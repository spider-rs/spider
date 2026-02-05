//! Dynamic skill loading for web automation.
//!
//! Skills are prompt fragments that provide specialized strategies for
//! specific challenge types (word search, tic-tac-toe, CAPTCHAs, etc.).
//! They get matched against page state and injected into the LLM context
//! via `system_prompt_extra`, keeping the default system prompt lean.
//!
//! # Architecture
//! ```text
//! ┌──────────────────┐     ┌──────────────────┐
//! │ DEFAULT_SYSTEM_   │     │  SkillRegistry   │
//! │ PROMPT (lean)     │     │  ┌─────────────┐ │
//! │ - action bindings │     │  │ grid_search  │ │
//! │ - JSON format     │     │  │ tic_tac_toe  │ │
//! │ - core strategy   │     │  │ text_captcha │ │
//! └────────┬─────────┘     │  │ slider_drag  │ │
//!          │                │  │ ...          │ │
//!          │                │  └──────┬──────┘ │
//!          │                └─────────┼────────┘
//!          ▼                          │ match(url, title, html)
//!   system_prompt_extra ◄─────────────┘
//! ```
//!
//! Skills can be:
//! - Embedded (built-in Rust constants)
//! - Loaded from local files (Markdown with YAML frontmatter)
//! - Fetched from URLs at runtime
//!
//! # Example
//! ```rust
//! use spider_agent::automation::skills::{SkillRegistry, Skill, SkillTrigger};
//!
//! let mut registry = SkillRegistry::new();
//!
//! // Add a custom skill
//! registry.add(Skill::new(
//!     "grid-selection",
//!     "Image grid selection challenge solver",
//! )
//! .with_trigger(SkillTrigger::title_contains("select all"))
//! .with_trigger(SkillTrigger::html_contains("grid-item"))
//! .with_content("For image grids, identify all matching tiles and click them..."));
//!
//! // Match skills against current page state
//! let context = registry.match_context("https://example.com", "Select all stop signs", "<div class='grid-item'>...");
//! // context contains the skill prompt to inject into system_prompt_extra
//! ```

use std::collections::HashMap;

// ─── S3 skill loading types ────────────────────────────────────────────────

/// Configuration for loading skills from an S3-compatible bucket.
#[cfg(feature = "skills_s3")]
#[derive(Debug, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct S3SkillSource {
    /// S3 bucket name.
    pub bucket: String,
    /// Folder prefix within the bucket, e.g. "tactics/".
    #[serde(default)]
    pub prefix: String,
    /// AWS region override. Defaults to SDK environment resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// Custom endpoint URL for S3-compatible stores (MinIO, R2, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,
    /// File extensions to load. Defaults to `["md"]`.
    #[serde(default = "default_s3_extensions")]
    pub extensions: Vec<String>,
}

#[cfg(feature = "skills_s3")]
fn default_s3_extensions() -> Vec<String> {
    vec!["md".to_string()]
}

#[cfg(feature = "skills_s3")]
impl S3SkillSource {
    /// Create a new S3 skill source with bucket and prefix.
    pub fn new(bucket: impl Into<String>, prefix: impl Into<String>) -> Self {
        Self {
            bucket: bucket.into(),
            prefix: prefix.into(),
            region: None,
            endpoint_url: None,
            extensions: default_s3_extensions(),
        }
    }

    /// Set the AWS region.
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Set a custom endpoint URL (for MinIO, R2, etc.).
    pub fn with_endpoint_url(mut self, url: impl Into<String>) -> Self {
        self.endpoint_url = Some(url.into());
        self
    }

    /// Set file extensions to load (e.g. `["md", "json"]`).
    pub fn with_extensions(mut self, exts: Vec<String>) -> Self {
        self.extensions = exts;
        self
    }
}

/// Errors from S3 skill loading.
#[cfg(feature = "skills_s3")]
#[derive(Debug)]
pub enum S3SkillError {
    /// AWS SDK error.
    Aws(String),
    /// No skills found in the specified bucket/prefix.
    NoSkillsFound,
    /// Failed to parse a skill file.
    ParseError {
        /// S3 object key that failed to parse.
        key: String,
        /// Reason for the parse failure.
        reason: String,
    },
}

#[cfg(feature = "skills_s3")]
impl std::fmt::Display for S3SkillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Aws(e) => write!(f, "AWS S3 error: {}", e),
            Self::NoSkillsFound => write!(f, "no skills found in S3 bucket"),
            Self::ParseError { key, reason } => {
                write!(f, "failed to parse skill '{}': {}", key, reason)
            }
        }
    }
}

#[cfg(feature = "skills_s3")]
impl std::error::Error for S3SkillError {}

/// A skill provides specialized context for solving specific challenge types.
///
/// Skills follow the [Agent Skills](https://github.com/anthropics/skills) pattern:
/// self-contained instruction sets with metadata for matching and loading.
#[derive(Debug, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Skill {
    /// Unique skill identifier (lowercase, hyphens for spaces).
    pub name: String,
    /// Description of what this skill handles and when to use it.
    pub description: String,
    /// Trigger conditions - if ANY match, the skill is activated.
    #[serde(default)]
    pub triggers: Vec<SkillTrigger>,
    /// The prompt content to inject when this skill is active.
    /// This gets appended to system_prompt_extra.
    pub content: String,
    /// Optional JavaScript to execute via `page.evaluate()` BEFORE the LLM
    /// sees the page. The JS should write results into `document.title` so the
    /// model can read them. This prevents the LLM from rewriting critical JS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_evaluate: Option<String>,
    /// Optional JavaScript code snippets the LLM can use.
    /// Keys are descriptive names, values are JS code strings.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub code_snippets: HashMap<String, String>,
    /// Priority: higher priority skills are injected first. Default 0.
    #[serde(default)]
    pub priority: i32,
}

impl Skill {
    /// Create a new skill with name and description.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            triggers: Vec::new(),
            content: String::new(),
            pre_evaluate: None,
            code_snippets: HashMap::new(),
            priority: 0,
        }
    }

    /// Add a trigger condition.
    pub fn with_trigger(mut self, trigger: SkillTrigger) -> Self {
        self.triggers.push(trigger);
        self
    }

    /// Set the prompt content.
    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = content.into();
        self
    }

    /// Set JavaScript to execute before the LLM sees the page.
    /// The JS should write results into `document.title`.
    pub fn with_pre_evaluate(mut self, js: impl Into<String>) -> Self {
        self.pre_evaluate = Some(js.into());
        self
    }

    /// Add a code snippet.
    pub fn with_snippet(
        mut self,
        name: impl Into<String>,
        code: impl Into<String>,
    ) -> Self {
        self.code_snippets.insert(name.into(), code.into());
        self
    }

    /// Set priority.
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Check if this skill matches the given page state.
    /// Returns true if ANY trigger matches.
    pub fn matches(&self, url: &str, title: &str, html: &str) -> bool {
        if self.triggers.is_empty() {
            return false;
        }
        self.triggers.iter().any(|t| t.matches(url, title, html))
    }

    /// Parse a skill from Markdown with YAML frontmatter.
    ///
    /// Format:
    /// ```markdown
    /// ---
    /// name: skill-name
    /// description: What this skill does
    /// triggers:
    ///   - title_contains: "some text"
    ///   - html_contains: "some-class"
    /// priority: 0
    /// ---
    ///
    /// # Skill content here
    /// Instructions for the LLM...
    /// ```
    pub fn from_markdown(markdown: &str) -> Option<Self> {
        let trimmed = markdown.trim();
        if !trimmed.starts_with("---") {
            return None;
        }

        // Find the closing ---
        let rest = &trimmed[3..];
        let end = rest.find("---")?;
        let frontmatter = &rest[..end].trim();
        let content = rest[end + 3..].trim();

        // Parse frontmatter as simple key-value pairs
        let mut name = String::new();
        let mut description = String::new();
        let mut triggers = Vec::new();
        let mut priority = 0i32;

        for line in frontmatter.lines() {
            let line = line.trim();
            if line.starts_with("name:") {
                name = line[5..].trim().trim_matches('"').to_string();
            } else if line.starts_with("description:") {
                description = line[12..].trim().trim_matches('"').to_string();
            } else if line.starts_with("priority:") {
                priority = line[9..].trim().parse().unwrap_or(0);
            } else if line.starts_with("- title_contains:") {
                let val = line[17..].trim().trim_matches('"').to_string();
                triggers.push(SkillTrigger::TitleContains(val));
            } else if line.starts_with("- url_contains:") {
                let val = line[15..].trim().trim_matches('"').to_string();
                triggers.push(SkillTrigger::UrlContains(val));
            } else if line.starts_with("- html_contains:") {
                let val = line[16..].trim().trim_matches('"').to_string();
                triggers.push(SkillTrigger::HtmlContains(val));
            }
        }

        if name.is_empty() {
            return None;
        }

        Some(Self {
            name,
            description,
            triggers,
            content: content.to_string(),
            pre_evaluate: None,
            code_snippets: HashMap::new(),
            priority,
        })
    }
}

/// Trigger conditions for skill activation.
#[derive(Debug, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum SkillTrigger {
    /// Match if page title contains this string (case-insensitive).
    TitleContains(String),
    /// Match if URL contains this string (case-insensitive).
    UrlContains(String),
    /// Match if HTML contains this string (case-insensitive).
    HtmlContains(String),
    /// Custom predicate: always matches (for manually-activated skills).
    Always,
}

impl SkillTrigger {
    /// Convenience: create a title-contains trigger.
    pub fn title_contains(s: impl Into<String>) -> Self {
        Self::TitleContains(s.into())
    }

    /// Convenience: create a URL-contains trigger.
    pub fn url_contains(s: impl Into<String>) -> Self {
        Self::UrlContains(s.into())
    }

    /// Convenience: create an HTML-contains trigger.
    pub fn html_contains(s: impl Into<String>) -> Self {
        Self::HtmlContains(s.into())
    }

    /// Check if this trigger matches the given page state.
    pub fn matches(&self, url: &str, title: &str, html: &str) -> bool {
        match self {
            Self::TitleContains(s) => title.to_lowercase().contains(&s.to_lowercase()),
            Self::UrlContains(s) => url.to_lowercase().contains(&s.to_lowercase()),
            Self::HtmlContains(s) => html.to_lowercase().contains(&s.to_lowercase()),
            Self::Always => true,
        }
    }
}

/// Registry for managing and matching skills.
///
/// Skills are matched against page state each round. When a skill matches,
/// its content is injected into `system_prompt_extra` for that round.
#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    skills: Vec<Skill>,
}

impl SkillRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a skill to the registry.
    pub fn add(&mut self, skill: Skill) {
        self.skills.push(skill);
    }

    /// Number of registered skills.
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Check if registry is empty.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Iterate over all skill names in the registry.
    pub fn skill_names(&self) -> impl Iterator<Item = &str> {
        self.skills.iter().map(|s| s.name.as_str())
    }

    /// Find all skills matching the current page state.
    pub fn find_matching(&self, url: &str, title: &str, html: &str) -> Vec<&Skill> {
        let mut matched: Vec<&Skill> = self
            .skills
            .iter()
            .filter(|s| s.matches(url, title, html))
            .collect();
        // Sort by priority (highest first)
        matched.sort_by(|a, b| b.priority.cmp(&a.priority));
        matched
    }

    /// Get the combined prompt context for all matching skills.
    ///
    /// Returns a string suitable for injection into `system_prompt_extra`.
    /// Returns empty string if no skills match.
    ///
    /// Uses default limits: max 3 skills, max 4000 chars total.
    /// For custom limits, use [`match_context_limited`].
    pub fn match_context(&self, url: &str, title: &str, html: &str) -> String {
        self.match_context_limited(url, title, html, 3, 4000)
    }

    /// Get combined prompt context with explicit limits.
    ///
    /// - `max_skills`: maximum number of skills to inject (highest priority first)
    /// - `max_chars`: maximum total characters for the combined skill context
    ///
    /// This prevents context bloat when many skills match. Skills are already
    /// sorted by priority (highest first), so lower-priority skills get dropped.
    pub fn match_context_limited(
        &self,
        url: &str,
        title: &str,
        html: &str,
        max_skills: usize,
        max_chars: usize,
    ) -> String {
        let matched = self.find_matching(url, title, html);
        if matched.is_empty() {
            return String::new();
        }

        let mut ctx = String::with_capacity(max_chars.min(matched.iter().map(|s| s.content.len() + 50).sum()));
        let mut count = 0;

        for skill in &matched {
            if count >= max_skills {
                break;
            }

            let entry = {
                let mut entry = String::new();
                if !ctx.is_empty() {
                    entry.push_str("\n\n");
                }
                entry.push_str("## Skill: ");
                entry.push_str(&skill.name);
                entry.push('\n');
                entry.push_str(&skill.content);

                // Include code snippets if any
                if !skill.code_snippets.is_empty() {
                    entry.push_str("\n\n### Available Code Snippets\n");
                    for (name, code) in &skill.code_snippets {
                        entry.push_str("**");
                        entry.push_str(name);
                        entry.push_str("**: `");
                        entry.push_str(code);
                        entry.push_str("`\n");
                    }
                }
                entry
            };

            // Check if adding this skill would exceed the char limit
            if ctx.len() + entry.len() > max_chars && !ctx.is_empty() {
                break;
            }

            ctx.push_str(&entry);
            count += 1;
        }

        ctx
    }

    /// Find all matching skills that have a `pre_evaluate` JS payload.
    /// Returns `(skill_name, js_code)` pairs sorted by priority.
    pub fn find_pre_evaluates(&self, url: &str, title: &str, html: &str) -> Vec<(&str, &str)> {
        self.find_matching(url, title, html)
            .into_iter()
            .filter_map(|s| {
                s.pre_evaluate
                    .as_deref()
                    .map(|js| (s.name.as_str(), js))
            })
            .collect()
    }

    /// Get a skill by name.
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    /// Remove a skill by name.
    pub fn remove(&mut self, name: &str) {
        self.skills.retain(|s| s.name != name);
    }

    /// Load a skill from a markdown string (with YAML frontmatter).
    pub fn load_markdown(&mut self, markdown: &str) -> Option<String> {
        let skill = Skill::from_markdown(markdown)?;
        let name = skill.name.clone();
        self.add(skill);
        Some(name)
    }

    /// Create a registry pre-loaded with built-in web challenge skills.
    ///
    /// These skills cover common interactive web challenge patterns:
    /// image grid selection, rotation puzzles, tic-tac-toe, word search, etc.
    /// Skills are only injected when their triggers match the current page state.
    pub fn with_builtin_web_challenges() -> Self {
        let mut registry = Self::new();

        // Image grid selection (e.g., "select all stop signs", "select all vegetables")
        registry.add(
            Skill::new("image-grid-selection", "Select matching images from a grid challenge")
                .with_trigger(SkillTrigger::html_contains("grid-item"))
                .with_trigger(SkillTrigger::html_contains("challenge-grid"))
                .with_trigger(SkillTrigger::title_contains("select all"))
                .with_priority(5)
                .with_content(IMAGE_GRID_SKILL)
        );

        // Rotation puzzle — pre_evaluate reads rotation state, model clicks tiles
        registry.add(
            Skill::new("rotation-puzzle", "Rotate an image or element to the correct orientation")
                .with_trigger(SkillTrigger::title_contains("rotat"))
                .with_trigger(SkillTrigger::html_contains("rotating-item"))
                .with_priority(5)
                .with_pre_evaluate(ROTATION_PRE_EVALUATE_JS)
                .with_content(ROTATION_SKILL_SIMPLIFIED)
        );

        // Tic-tac-toe / XOXO — high priority to override image-grid when both match
        // Uses pre_evaluate to run board-reading + solver JS before the LLM sees
        // the page. The model only sees the result in document.title and clicks.
        registry.add(
            Skill::new("tic-tac-toe", "Play tic-tac-toe (noughts and crosses) game")
                .with_trigger(SkillTrigger::title_contains("xoxo"))
                .with_trigger(SkillTrigger::title_contains("tic-tac"))
                .with_trigger(SkillTrigger::title_contains("tic tac"))
                .with_trigger(SkillTrigger::html_contains("tic-tac"))
                .with_trigger(SkillTrigger::html_contains("cell-selected"))
                .with_trigger(SkillTrigger::html_contains("cell-disabled"))
                .with_priority(10)
                .with_pre_evaluate(TTT_PRE_EVALUATE_JS)
                .with_content(TTT_SKILL_SIMPLIFIED)
        );

        // Word search — higher priority than image-grid since word-search pages also have grid-item
        // Uses pre_evaluate to run grid extraction + solver JS before the LLM sees
        // the page. The model only sees found word coordinates in document.title.
        registry.add(
            Skill::new("word-search", "Find and select words in a letter grid")
                .with_trigger(SkillTrigger::title_contains("word search"))
                .with_trigger(SkillTrigger::title_contains("wordsearch"))
                .with_trigger(SkillTrigger::html_contains("word-search-grid-item"))
                .with_trigger(SkillTrigger::html_contains("word-search"))
                .with_priority(8)
                .with_pre_evaluate(WORD_SEARCH_PRE_EVALUATE_JS)
                .with_content(WORD_SEARCH_SKILL_SIMPLIFIED)
        );

        // Text CAPTCHA / math challenges / distorted text
        // Triggers are specific to text-input captchas to avoid matching on grid/rotation levels
        // that also have "captcha" in class names (e.g., #captcha-verify-button).
        registry.add(
            Skill::new("text-captcha", "Solve text-based CAPTCHAs, distorted text, and math challenges")
                .with_trigger(SkillTrigger::html_contains("captcha-input"))
                .with_trigger(SkillTrigger::html_contains("captcha-text"))
                .with_trigger(SkillTrigger::title_contains("wiggles"))
                .with_priority(3)
                .with_content(TEXT_CAPTCHA_SKILL)
        );

        // Slider / drag challenges
        registry.add(
            Skill::new("slider-drag", "Solve slider and drag-to-position challenges")
                .with_trigger(SkillTrigger::html_contains("slider-track"))
                .with_trigger(SkillTrigger::html_contains("slider-handle"))
                .with_trigger(SkillTrigger::html_contains("range-slider"))
                .with_priority(4)
                .with_content(SLIDER_DRAG_SKILL)
        );

        // Checkbox / simple click challenges (L1 of not-a-robot)
        registry.add(
            Skill::new("checkbox-click", "Click a checkbox to prove you are human")
                .with_trigger(SkillTrigger::html_contains("captcha-checkbox"))
                .with_trigger(SkillTrigger::html_contains("checkbox-label"))
                .with_priority(2)
                .with_content(CHECKBOX_SKILL)
        );

        registry
    }
}

// ─── S3 skill loading impl ─────────────────────────────────────────────────

#[cfg(feature = "skills_s3")]
impl SkillRegistry {
    /// Load skills from an S3-compatible bucket.
    ///
    /// Lists objects under `source.prefix`, filters by `source.extensions`,
    /// downloads each, and parses via `Skill::from_markdown()` for `.md` files
    /// or `serde_json::from_str` for `.json` files.
    ///
    /// Name conflicts: S3 skills replace any existing skill with the same name.
    ///
    /// Returns the count of successfully loaded skills.
    pub async fn load_from_s3(&mut self, source: &S3SkillSource) -> Result<usize, S3SkillError> {
        let sdk_config = {
            let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
            if let Some(ref region) = source.region {
                loader = loader.region(aws_config::Region::new(region.clone()));
            }
            if let Some(ref endpoint) = source.endpoint_url {
                loader = loader.endpoint_url(endpoint);
            }
            loader.load().await
        };

        let client = aws_sdk_s3::Client::new(&sdk_config);

        let mut continuation_token: Option<String> = None;
        let mut loaded = 0usize;
        let exts: Vec<&str> = source.extensions.iter().map(|s| s.as_str()).collect();

        loop {
            let mut req = client
                .list_objects_v2()
                .bucket(&source.bucket)
                .prefix(&source.prefix);

            if let Some(ref token) = continuation_token {
                req = req.continuation_token(token);
            }

            let resp = req
                .send()
                .await
                .map_err(|e| S3SkillError::Aws(e.to_string()))?;

            let contents = resp.contents();
            {
                for obj in contents {
                    let key = match obj.key() {
                        Some(k) => k,
                        None => continue,
                    };

                    // Filter by extension
                    let matches_ext = exts.iter().any(|ext| {
                        key.ends_with(&format!(".{}", ext))
                    });
                    if !matches_ext {
                        continue;
                    }

                    // Download the object
                    let get_resp = client
                        .get_object()
                        .bucket(&source.bucket)
                        .key(key)
                        .send()
                        .await
                        .map_err(|e| S3SkillError::Aws(e.to_string()))?;

                    let body = get_resp
                        .body
                        .collect()
                        .await
                        .map_err(|e| S3SkillError::Aws(e.to_string()))?;

                    let text = String::from_utf8_lossy(&body.into_bytes()).into_owned();

                    // Parse based on extension
                    let skill = if key.ends_with(".json") {
                        serde_json::from_str::<Skill>(&text).map_err(|e| {
                            S3SkillError::ParseError {
                                key: key.to_string(),
                                reason: e.to_string(),
                            }
                        })?
                    } else {
                        // Markdown
                        Skill::from_markdown(&text).ok_or_else(|| S3SkillError::ParseError {
                            key: key.to_string(),
                            reason: "invalid markdown frontmatter".to_string(),
                        })?
                    };

                    // Replace existing skill with same name
                    self.remove(&skill.name);
                    self.add(skill);
                    loaded += 1;
                }
            }

            if resp.is_truncated() == Some(true) {
                continuation_token = resp.next_continuation_token().map(|s| s.to_string());
            } else {
                break;
            }
        }

        if loaded == 0 {
            return Err(S3SkillError::NoSkillsFound);
        }

        Ok(loaded)
    }

    /// Create a registry with built-in web challenge skills merged with S3 skills.
    ///
    /// S3 skills override built-in skills with the same name.
    pub async fn with_builtin_and_s3(source: &S3SkillSource) -> Result<Self, S3SkillError> {
        let mut registry = Self::with_builtin_web_challenges();
        registry.load_from_s3(source).await?;
        Ok(registry)
    }
}

// ─── Built-in skill content ──────────────────────────────────────────────

const IMAGE_GRID_SKILL: &str = r##"
Image grid selection (e.g., "select all stop signs"):
**(Skip if tic-tac-toe/XOXO or Word Search — use those skills instead.)**

**RULES:**
- Use REAL Click actions ONLY (never el.click() in Evaluate)
- Clicking toggles selection. nth-child is 1-indexed.
- **SOLVE IN 2 ROUNDS MAX. If verify fails twice → REFRESH and retry:**
  `[{"Click":".captcha-refresh"},{"Wait":1000}]`

**Round 1:** Look at screenshot carefully. Click ALL matching tiles + verify in ONE step list:
```json
"steps": [
  {"Click":".grid-item:nth-child(3)"},
  {"Click":".grid-item:nth-child(7)"},
  {"Wait":300},
  {"Click":"#captcha-verify-button"}
]
```

**Round 2 (if still same level):** Toggle wrong tiles (deselect non-matches, select missed ones), then verify.

**Round 3+:** Refresh the captcha: `[{"Click":".captcha-refresh"},{"Wait":1000}]`
"##;

/// JS executed by the engine before the LLM sees the rotation page.
/// Reads each tile's CSS transform, computes clicks needed, writes to title.
/// Does NOT click — the model uses real Click actions based on the title info.
const ROTATION_PRE_EVALUATE_JS: &str = "try{const t=[...document.querySelectorAll('.rotating-item')];const n=t.length;const tiles=t.map((e,i)=>{const m=getComputedStyle(e).transform;let c=0;if(m&&m!=='none'){const v=m.match(/matrix\\(([^)]+)\\)/);if(v){const p=v[1].split(',').map(Number);const a=Math.round(Math.atan2(p[1],p[0])*180/Math.PI);c=a>45&&a<135?3:Math.abs(a)>135?2:a<-45&&a>-135?1:0;}}return{i,c};});const done=tiles.every(t=>t.c===0);const clicks=tiles.filter(t=>t.c>0).map(t=>t.i+':'+t.c).join(',');document.title='ROT:'+JSON.stringify({n,done,clicks});}catch(e){document.title='ROT_ERR:'+e.message;}";

/// Simplified rotation skill content — pre_evaluate reads state, model clicks tiles.
const ROTATION_SKILL_SIMPLIFIED: &str = r##"
Rotation puzzle: tiles form an image, some rotated 90/180/270°. Each click = +90° clockwise.
Read `document.title` for auto-detected rotation state.

Title format: `ROT:{"n":9,"done":false,"clicks":"1:1,2:2,6:3,7:2,8:3"}`
- `n` = number of tiles
- `done` = true when all tiles are upright → click verify
- `clicks` = "tileIndex:clicksNeeded" pairs (0-indexed tiles, use nth-child = index+1)

**Rules:**
- **done is true** → all upright, verify: `[{"Click":"#captcha-verify-button"}]`
- **done is false** → click each tile the number of times shown. Use `.rotating-item:nth-child(N)` (N = index+1, 1-indexed):
```json
"steps": [
  {"Click":".rotating-item:nth-child(2)"},
  {"Click":".rotating-item:nth-child(3)"},{"Click":".rotating-item:nth-child(3)"},
  {"Click":".rotating-item:nth-child(7)"},{"Click":".rotating-item:nth-child(7)"},{"Click":".rotating-item:nth-child(7)"},
  {"Wait":500},
  {"Click":"#captcha-verify-button"}
]
```
- **ROT_ERR or n is 0** → wait: `[{"Wait":1000}]`
- After verify, if still on rotation level, refresh and retry: `[{"Click":".captcha-refresh"},{"Wait":800}]`

**Do NOT write any Evaluate JS. Rotation state is auto-detected.**
"##;

/// JS executed by the engine before the LLM sees the TTT page.
/// Tracks our moves in a persistent hidden DOM element (`#ttt-h`) to distinguish
/// our marks (M) from opponent marks (T). CSS classes are temporal (cell-selected =
/// last move, cell-disabled = older), so we can't use them for player identity.
/// Uses proper win/block strategy and clicks via dispatchEvent.
const TTT_PRE_EVALUATE_JS: &str = "try{const cells=[...document.querySelectorAll('.grid-item')].filter(el=>el.offsetWidth>20&&el.offsetHeight>20);const n=cells.length;const occ=new Set();cells.forEach((el,i)=>{const inner=el.querySelector('.tic-tac-toe-cell');if(inner&&(inner.className.includes('cell-selected')||inner.className.includes('cell-disabled')))occ.add(i);});let tr=document.getElementById('ttt-h');if(!tr){tr=document.createElement('div');tr.id='ttt-h';tr.style.display='none';tr.dataset.m='';document.body.appendChild(tr);}const my=new Set(tr.dataset.m?tr.dataset.m.split(',').map(Number):[]);if(occ.size===0)my.clear();for(const m of my)if(!occ.has(m))my.delete(m);const opp=new Set([...occ].filter(i=>!my.has(i)));const board=Array.from({length:9},(_,i)=>my.has(i)?'M':opp.has(i)?'T':'.');const W=[[0,1,2],[3,4,5],[6,7,8],[0,3,6],[1,4,7],[2,5,8],[0,4,8],[2,4,6]];let myWin=false,thWin=false;for(const w of W){if(w.every(i=>board[i]==='M'))myWin=true;if(w.every(i=>board[i]==='T'))thWin=true;}let best=-1;if(!myWin&&!thWin){for(const w of W){const mi=w.filter(i=>board[i]==='M'),e=w.filter(i=>board[i]==='.');if(mi.length===2&&e.length===1){best=e[0];break;}}if(best<0)for(const w of W){const ti=w.filter(i=>board[i]==='T'),e=w.filter(i=>board[i]==='.');if(ti.length===2&&e.length===1){best=e[0];break;}}if(best<0&&board[4]==='.')best=4;if(best<0)for(const c of[0,2,6,8])if(board[c]==='.'){best=c;break;}if(best<0)for(const c of[1,3,5,7])if(board[c]==='.'){best=c;break;}}let clicked=false;if(best>=0&&cells[best]){const el=cells[best];const r=el.getBoundingClientRect();const ev={bubbles:true,cancelable:true,clientX:r.x+r.width/2,clientY:r.y+r.height/2};el.dispatchEvent(new PointerEvent('pointerdown',ev));el.dispatchEvent(new MouseEvent('mousedown',ev));el.dispatchEvent(new PointerEvent('pointerup',ev));el.dispatchEvent(new MouseEvent('mouseup',ev));el.dispatchEvent(new MouseEvent('click',ev));clicked=true;my.add(best);board[best]='M';for(const w of W)if(w.every(i=>board[i]==='M'))myWin=true;}tr.dataset.m=[...my].join(',');const full=!board.includes('.');document.title='TTT:'+JSON.stringify({n,board:board.join(''),best,clicked,myWin,thWin,full});}catch(e){document.title='TTT_ERR:'+e.message;}";

/// Simplified TTT skill content — pre_evaluate handles clicking, model just checks outcome.
const TTT_SKILL_SIMPLIFIED: &str = r##"
Tic-tac-toe (XOXO): Board state is tracked across rounds. Moves are auto-made via dispatchEvent.
Read `document.title` for result. M = my mark, T = opponent mark, . = empty.

**Ignore image-grid-selection skill — this is NOT an image grid.**

Title format: `TTT:{"n":9,"board":"M..T.M..T","best":4,"clicked":true,"myWin":false,"thWin":false,"full":false}`

**Rules (check title EVERY round):**
- **myWin is true** → we won! Click verify: `[{"Click":"#captcha-verify-button"}]`
- **thWin is true** → opponent won, refresh: `[{"Click":".captcha-refresh"},{"Wait":800}]`
- **clicked is true, no winner** → wait for opponent: `[{"Wait":800}]`
- **full is true, no winner** → draw, refresh: `[{"Click":".captcha-refresh"},{"Wait":800}]`
- **best is -1, not full** → wait for state: `[{"Wait":800}]`
- **n != 9 or TTT_ERR** → board not ready, wait: `[{"Wait":1000}]`

**Do NOT write any Evaluate JS or use ClickPoint. Moves are made automatically.**
"##;

/// JS executed by the engine before the LLM sees the Word Search page.
/// Extracts grid + finds words algorithmically. If words found, provides drag coordinates.
/// If words NOT found (selectors miss), provides grid text so model can solve visually.
/// Does NOT overwrite title with empty data if model has already set useful info.
const WORD_SEARCH_PRE_EVALUATE_JS: &str = "try{let cells=[...document.querySelectorAll('.word-search-grid-item')];if(!cells.length)cells=[...document.querySelectorAll('.grid-item')].filter(el=>el.textContent.trim().length===1);if(!cells.length)cells=[...document.querySelectorAll('[class*=letter]')].filter(el=>el.textContent.trim().length===1);const n=cells.length;if(n<4){document.title='WS:'+JSON.stringify({n,err:'no_grid'});}else{const rects=cells.map(c=>{const r=c.getBoundingClientRect();return{x:Math.round(r.x+r.width/2),y:Math.round(r.y+r.height/2)};});const letters=cells.map(c=>c.textContent.trim().toUpperCase());const tops=[...new Set(rects.map(r=>r.y))].sort((a,b)=>a-b);const rows=tops.length||1,cols=Math.round(n/rows)||1;const grid=[];for(let r=0;r<rows;r++)grid.push(letters.slice(r*cols,(r+1)*cols));let wordEls=[...document.querySelectorAll('.word-search-words span,.word-search-word')];if(!wordEls.length)wordEls=[...document.querySelectorAll('[class*=word-item],[class*=clue]')];if(!wordEls.length)wordEls=[...document.querySelectorAll('.word-search-words li,.words li')];if(!wordEls.length)wordEls=[...document.querySelectorAll('.words span')];if(!wordEls.length){const all=[...document.querySelectorAll('span,div')].filter(el=>!el.querySelector('*')&&el.textContent.trim().match(/^[A-Z\\s]{3,20}$/i)&&!el.closest('.grid-item,.word-search-grid-item'));wordEls=all;}const words=wordEls.map(el=>el.textContent.trim().toUpperCase().replace(/\\s+/g,'')).filter(w=>w.length>1&&w.length<=20&&w.match(/^[A-Z]+$/));const dirs=[[0,1],[0,-1],[1,0],[-1,0],[1,1],[1,-1],[-1,1],[-1,-1]];const found={};words.forEach(w=>{for(let r=0;r<rows;r++)for(let c=0;c<cols;c++)for(const[dr,dc]of dirs){let ok=true;for(let k=0;k<w.length;k++){const nr=r+dr*k,nc=c+dc*k;if(nr<0||nr>=rows||nc<0||nc>=cols||grid[nr][nc]!==w[k]){ok=false;break;}}if(ok){const si=r*cols+c,ei=(r+dr*(w.length-1))*cols+(c+dc*(w.length-1));found[w]={from:rects[si],to:rects[ei]};return;}}});if(Object.keys(found).length>0){document.title='WS:'+JSON.stringify({n,rows,cols,words,found});}else{const gt=grid.map(r=>r.join('')).join('/');document.title='WS:'+JSON.stringify({n,rows,cols,grid:gt,words:[],found:{}});}}}catch(e){document.title='WS_ERR:'+e.message;}";

/// Simplified Word Search skill content — model reads pre-computed title and drags.
const WORD_SEARCH_SKILL_SIMPLIFIED: &str = r##"
Word search puzzle: Grid is auto-analyzed. Read `document.title` for result.

**(Skip image-grid-selection skill — this is a word search, NOT an image grid.)**

Title formats:
- With found words: `WS:{"n":100,"rows":10,"cols":10,"words":["STOP"],"found":{"STOP":{"from":{"x":100,"y":200},"to":{"x":300,"y":200}}}}`
- Grid only (words not auto-detected): `WS:{"n":100,"rows":10,"cols":10,"grid":"CRNW.../PBRB...","words":[],"found":{}}`

**Rules:**
1. If `found` has entries → drag each word using ClickDragPoint, then verify:
```json
"steps": [
  {"ClickDragPoint":{"from_x":100,"from_y":200,"to_x":300,"to_y":200}},
  {"Wait":300},
  {"ClickDragPoint":{"from_x":150,"from_y":300,"to_x":150,"to_y":500}},
  {"Wait":300},
  {"Click":"#captcha-verify-button"}
]
```
2. If `words:[]` (words not auto-detected) → use Evaluate to find words and solve:
   - Read the words to find from the page (look for word list elements)
   - Search the grid algorithmically for each word
   - Get cell bounding rects and use ClickDragPoint
3. If `n` is 0 or `WS_ERR` → wait: `[{"Wait":1000}]`
4. After verify, if still on word search, refresh: `[{"Click":".captcha-refresh"},{"Wait":800}]`
"##;

const TEXT_CAPTCHA_SKILL: &str = r##"
Distorted text CAPTCHA: Read the wiggling/wavy text from the SCREENSHOT and type it.

**DO NOT analyze canvas pixels or write pixel-scanning JS. Just READ the text visually from the screenshot.**
The answer is 4-8 uppercase letters shown in a wavy/distorted style. Do NOT type labels like "HUMAN".

**Steps (solve in 1 round):**
```json
"steps": [
  {"Clear":".captcha-input-text"},
  {"Fill":{"selector":".captcha-input-text","value":"YOURTEXT"}},
  {"Click":".captcha-button"}
]
```

If wrong: swap most ambiguous char (O↔D↔0, S↔5, I↔1↔L, Z↔2, B↔8).
**After 2 fails → refresh:** `[{"Click":"img.captcha-refresh"},{"Wait":1500}]`
If `img.captcha-refresh` fails, use ClickPoint on the small refresh icon near the captcha.
Never submit same text twice.
"##;

const CHECKBOX_SKILL: &str = r##"
Simple checkbox challenge: Click the checkbox to pass.

**Steps:**
```json
"steps": [{"Click":".captcha-checkbox"},{"Wait":500},{"Click":"#captcha-verify-button"}]
```
If `.captcha-checkbox` fails, try ClickPoint on the checkbox visible in the screenshot.
Solve in 1 round.
"##;

const SLIDER_DRAG_SKILL: &str = r#"
Strategy for slider and drag-to-position challenges:

1. **Identify the slider element** using Evaluate:
   ```js
   document.title = JSON.stringify({
     sliders: [...document.querySelectorAll('input[type=range], [class*=slider], [role=slider]')].map(el => ({
       tag: el.tagName, cls: el.className, min: el.min, max: el.max, step: el.step, val: el.value,
       rect: el.getBoundingClientRect()
     })),
     handles: [...document.querySelectorAll('[class*=handle], [class*=thumb], [class*=knob]')].map(el => ({
       cls: el.className, rect: el.getBoundingClientRect()
     }))
   })
   ```
2. **For range inputs**: Fill with target value, then dispatch events:
   ```js
   const el = document.querySelector('input[type=range]');
   el.value = TARGET;
   el.dispatchEvent(new Event('input', {bubbles:true}));
   el.dispatchEvent(new Event('change', {bubbles:true}));
   ```
3. **For custom sliders**: Use ClickDragPoint from handle position to target position
4. **Calculate target position**: `targetX = trackLeft + (targetValue - min) / (max - min) * trackWidth`
5. **Verify position** after dragging, adjust if needed
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_trigger_matching() {
        let skill = Skill::new("grid-selection", "Select images from grid")
            .with_trigger(SkillTrigger::title_contains("select all"))
            .with_trigger(SkillTrigger::html_contains("grid-item"))
            .with_content("Select tiles matching the description...");

        // Title match
        assert!(skill.matches("https://example.com", "Select all stop signs", ""));
        // HTML match
        assert!(skill.matches("https://example.com", "", "<div class='grid-item'>"));
        // No match
        assert!(!skill.matches("https://example.com", "Home page", "<div>hello</div>"));
    }

    #[test]
    fn test_skill_registry_matching() {
        let mut registry = SkillRegistry::new();

        registry.add(
            Skill::new("grid", "Grid challenges")
                .with_trigger(SkillTrigger::html_contains("grid-item"))
                .with_content("Grid strategy here")
                .with_priority(1),
        );

        registry.add(
            Skill::new("ttt", "Tic-tac-toe")
                .with_trigger(SkillTrigger::title_contains("xoxo"))
                .with_content("TTT strategy here")
                .with_priority(2),
        );

        // Only grid matches
        let ctx = registry.match_context("", "", "<div class='grid-item'>");
        assert!(ctx.contains("Grid strategy"));
        assert!(!ctx.contains("TTT strategy"));

        // Only TTT matches
        let ctx = registry.match_context("", "XOXO Game", "<div>board</div>");
        assert!(ctx.contains("TTT strategy"));
        assert!(!ctx.contains("Grid strategy"));

        // Neither matches
        let ctx = registry.match_context("", "Home", "<div>hello</div>");
        assert!(ctx.is_empty());
    }

    #[test]
    fn test_skill_from_markdown() {
        let md = r#"---
name: word-search
description: Word search grid solver
priority: 5
triggers:
  - title_contains: word search
  - html_contains: grid-item
---

# Word Search Strategy

Find words in the grid by extracting letters and searching algorithmically.
Use Evaluate to click found cells programmatically."#;

        let skill = Skill::from_markdown(md).unwrap();
        assert_eq!(skill.name, "word-search");
        assert_eq!(skill.description, "Word search grid solver");
        assert_eq!(skill.priority, 5);
        assert_eq!(skill.triggers.len(), 2);
        assert!(skill.content.contains("Word Search Strategy"));
        assert!(skill.content.contains("algorithmically"));
    }

    #[test]
    fn test_skill_no_triggers_never_matches() {
        let skill = Skill::new("empty", "No triggers").with_content("content");
        assert!(!skill.matches("", "", ""));
    }

    #[test]
    fn test_always_trigger() {
        let skill = Skill::new("always", "Always active")
            .with_trigger(SkillTrigger::Always)
            .with_content("Always injected");

        assert!(skill.matches("", "", ""));
        assert!(skill.matches("any", "any", "any"));
    }

    #[test]
    fn test_builtin_web_challenges() {
        let registry = SkillRegistry::with_builtin_web_challenges();
        assert!(registry.len() >= 7);

        // Image grid should match on grid-item class
        let ctx = registry.match_context("", "", "<div class='grid-item'>img</div>");
        assert!(ctx.contains("image-grid-selection"));

        // Tic-tac-toe should match on XOXO title
        let ctx = registry.match_context("", "XOXO Game", "");
        assert!(ctx.contains("tic-tac-toe"));

        // Word search should match on title
        let ctx = registry.match_context("", "Word Search Puzzle", "");
        assert!(ctx.contains("word-search"));

        // Rotation should match on rotating-item class
        let ctx = registry.match_context("", "", "<div class='rotating-item'>");
        assert!(ctx.contains("rotation-puzzle"));

        // No match on unrelated page
        let ctx = registry.match_context("https://example.com", "Home", "<div>hello</div>");
        assert!(ctx.is_empty());
    }

    #[test]
    fn test_pre_evaluate_field() {
        // Default is None
        let skill = Skill::new("test", "test skill");
        assert!(skill.pre_evaluate.is_none());

        // Builder sets it
        let skill = Skill::new("test", "test skill")
            .with_pre_evaluate("document.title='HELLO'");
        assert_eq!(skill.pre_evaluate.as_deref(), Some("document.title='HELLO'"));

        // from_markdown leaves it None
        let md = "---\nname: foo\n---\ncontent";
        let skill = Skill::from_markdown(md).unwrap();
        assert!(skill.pre_evaluate.is_none());
    }

    #[test]
    fn test_find_pre_evaluates() {
        let mut registry = SkillRegistry::new();

        // Skill with pre_evaluate
        registry.add(
            Skill::new("solver", "Auto-solver")
                .with_trigger(SkillTrigger::title_contains("game"))
                .with_pre_evaluate("document.title='SOLVED'")
                .with_content("Read the title.")
                .with_priority(5),
        );

        // Skill without pre_evaluate
        registry.add(
            Skill::new("helper", "Manual helper")
                .with_trigger(SkillTrigger::title_contains("game"))
                .with_content("Use Evaluate to read...")
                .with_priority(3),
        );

        // Both match, but only one has pre_evaluate
        let pre_evals = registry.find_pre_evaluates("", "game level", "");
        assert_eq!(pre_evals.len(), 1);
        assert_eq!(pre_evals[0].0, "solver");
        assert_eq!(pre_evals[0].1, "document.title='SOLVED'");

        // No match
        let pre_evals = registry.find_pre_evaluates("", "home", "");
        assert!(pre_evals.is_empty());
    }

    #[test]
    fn test_builtin_ttt_has_pre_evaluate() {
        let registry = SkillRegistry::with_builtin_web_challenges();
        let ttt = registry.get("tic-tac-toe").expect("tic-tac-toe skill missing");
        assert!(ttt.pre_evaluate.is_some(), "TTT should have pre_evaluate JS");
        let js = ttt.pre_evaluate.as_deref().unwrap();
        assert!(js.contains("TTT:"), "TTT pre_evaluate should set title with TTT: prefix");
        assert!(js.contains("cell-selected"), "TTT pre_evaluate should use cell-selected selector");
        assert!(js.contains("cell-disabled"), "TTT pre_evaluate should use cell-disabled selector");
        assert!(js.contains("TTT_ERR"), "TTT pre_evaluate should have error handling");
    }

    #[test]
    fn test_builtin_word_search_has_pre_evaluate() {
        let registry = SkillRegistry::with_builtin_web_challenges();
        let ws = registry.get("word-search").expect("word-search skill missing");
        assert!(ws.pre_evaluate.is_some(), "Word search should have pre_evaluate JS");
        let js = ws.pre_evaluate.as_deref().unwrap();
        assert!(js.contains("WS:"), "WS pre_evaluate should set title with WS: prefix");
        assert!(js.contains("word-search-grid-item"), "WS pre_evaluate should use word-search-grid-item selector");
        assert!(js.contains("WS_ERR"), "WS pre_evaluate should have error handling");
    }

    #[test]
    fn test_builtin_ttt_triggers_fixed() {
        let registry = SkillRegistry::with_builtin_web_challenges();
        let ttt = registry.get("tic-tac-toe").unwrap();

        // Should match on cell-selected (the correct DOM class)
        assert!(ttt.matches("", "", "<div class='cell-selected'>"));
        // Should match on cell-disabled (the correct DOM class)
        assert!(ttt.matches("", "", "<div class='cell-disabled'>"));
        // Should match on XOXO title
        assert!(ttt.matches("", "XOXO", ""));
    }

    #[test]
    fn test_simplified_skills_no_evaluate_js() {
        let registry = SkillRegistry::with_builtin_web_challenges();

        // TTT simplified content should NOT contain Evaluate JS
        let ttt = registry.get("tic-tac-toe").unwrap();
        assert!(ttt.content.contains("Do NOT write any Evaluate JS"));
        assert!(!ttt.content.contains("querySelectorAll"));

        // Word search may allow Evaluate as fallback when words aren't auto-detected
        let ws = registry.get("word-search").unwrap();
        assert!(ws.content.contains("ClickDragPoint"), "WS skill should mention ClickDragPoint");
        assert!(ws.pre_evaluate.is_some(), "WS should have pre_evaluate JS");

        // Rotation simplified content should NOT contain Evaluate JS
        let rot = registry.get("rotation-puzzle").unwrap();
        assert!(rot.content.contains("Do NOT write any Evaluate JS"));
        assert!(!rot.content.contains("querySelectorAll"));
        assert!(rot.pre_evaluate.is_some(), "Rotation should have pre_evaluate JS");
    }

    #[test]
    fn test_skills_without_pre_evaluate_unchanged() {
        let registry = SkillRegistry::with_builtin_web_challenges();

        // Image grid, text-captcha, slider, checkbox should NOT have pre_evaluate
        for name in &["image-grid-selection", "text-captcha", "slider-drag", "checkbox-click"] {
            let skill = registry.get(name).unwrap_or_else(|| panic!("{} missing", name));
            assert!(
                skill.pre_evaluate.is_none(),
                "{} should NOT have pre_evaluate",
                name
            );
        }
    }
}
