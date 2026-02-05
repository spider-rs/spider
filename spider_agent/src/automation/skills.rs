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

        // Rotation puzzle
        registry.add(
            Skill::new("rotation-puzzle", "Rotate an image or element to the correct orientation")
                .with_trigger(SkillTrigger::title_contains("rotat"))
                .with_trigger(SkillTrigger::html_contains("rotate"))
                .with_trigger(SkillTrigger::html_contains("slider"))
                .with_priority(5)
                .with_content(ROTATION_SKILL)
        );

        // Tic-tac-toe / XOXO
        registry.add(
            Skill::new("tic-tac-toe", "Play tic-tac-toe (noughts and crosses) game")
                .with_trigger(SkillTrigger::title_contains("xoxo"))
                .with_trigger(SkillTrigger::title_contains("tic-tac"))
                .with_trigger(SkillTrigger::title_contains("tic tac"))
                .with_trigger(SkillTrigger::html_contains("tic-tac"))
                .with_priority(5)
                .with_content(TIC_TAC_TOE_SKILL)
        );

        // Word search
        registry.add(
            Skill::new("word-search", "Find and select words in a letter grid")
                .with_trigger(SkillTrigger::title_contains("word search"))
                .with_trigger(SkillTrigger::title_contains("wordsearch"))
                .with_priority(5)
                .with_content(WORD_SEARCH_SKILL)
        );

        // Text CAPTCHA / math challenges / distorted text
        registry.add(
            Skill::new("text-captcha", "Solve text-based CAPTCHAs, distorted text, and math challenges")
                .with_trigger(SkillTrigger::html_contains("captcha"))
                .with_trigger(SkillTrigger::title_contains("captcha"))
                .with_trigger(SkillTrigger::title_contains("wiggles"))
                .with_trigger(SkillTrigger::title_contains("verify"))
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

        registry
    }
}

// ─── Built-in skill content ──────────────────────────────────────────────

const IMAGE_GRID_SKILL: &str = r##"
Strategy for image grid selection challenges (e.g., "select all stop signs"):

**GOAL: Solve in 2 rounds max. Round 1: select + verify. Round 2: adjust if wrong.**

1. **Look at the screenshot** carefully. Identify which tiles contain the target object by position (row, column).
2. **Use Evaluate to toggle tiles to the correct state in ONE step**:
   ```js
   const items = [...document.querySelectorAll('[class*=grid-item], [class*=grid] > *')];
   const correct = new Set([0, 1, 4, 5]); // replace with YOUR correct indices
   items.forEach((el, i) => {
     const sel = el.classList.contains('selected') || el.classList.contains('grid-item-selected');
     if (correct.has(i) !== sel) el.click(); // only toggle tiles in wrong state
   });
   document.title = 'DONE';
   ```
3. **Click Verify** in the same round after the Evaluate.
4. **If wrong**: try DIFFERENT tile indices. Don't repeat the same selection.

Key rules:
- Toggle only tiles that need changing, never deselect-all then reselect.
- From the screenshot, map tile positions to grid indices (left-to-right, top-to-bottom, 0-indexed).
- If stuck, use Evaluate to read alt text or image src hints: `items.map((el,i)=>({i, alt:el.querySelector('img')?.alt}))`
"##;

const ROTATION_SKILL: &str = r#"
Strategy for rotation/orientation challenges:

**GOAL: Solve in 1-2 rounds. Read rotation state, set correct value, submit.**

Round 1 - Read and fix in one shot:
1. **Use Evaluate to read current rotation AND set the correct value**:
   ```js
   const slider = document.querySelector('input[type=range], [class*=slider], [role=slider]');
   const rotated = document.querySelector('[style*=rotate]');
   const match = rotated?.style.cssText.match(/rotate\((-?\d+\.?\d*)deg\)/);
   const currentDeg = match ? parseFloat(match[1]) : 0;
   const targetSlider = slider ? Math.round((360 - (currentDeg % 360 + 360) % 360) % 360 / 360 * (slider.max - slider.min) + Number(slider.min)) : 0;
   if (slider) { slider.value = targetSlider; slider.dispatchEvent(new Event('input',{bubbles:true})); slider.dispatchEvent(new Event('change',{bubbles:true})); }
   document.title = 'ROT:' + currentDeg + ' TARGET_SLIDER:' + targetSlider;
   ```
2. **Click Verify/Submit** in the same round.
3. If the image doesn't look upright, use the screenshot to estimate degrees off and adjust.

Key rules:
- Read the rotation from `transform: rotate(Xdeg)` style.
- For range sliders: set value programmatically via Evaluate with events.
- For drag handles: use ClickDragPoint.
- Don't spend rounds just reading - read and act together.
"#;

const TIC_TAC_TOE_SKILL: &str = r#"
Strategy for tic-tac-toe (XOXO) challenges:

**GOAL: Play optimally with 1 Evaluate + 1 Click per move. Read board, decide, act - all in one round.**

Each round:
1. **Read board + decide + click in one round**. Use Evaluate to read board state:
   ```js
   const cells = [...document.querySelectorAll('[class*=square], [class*=cell], [class*=tile], td')];
   const board = cells.map(el => el.textContent.trim() || (el.querySelector('[class*=x]') ? 'X' : el.querySelector('[class*=o]') ? 'O' : ''));
   document.title = 'BOARD:' + board.join(',');
   ```
2. **In the SAME round**, click the best empty cell immediately. Don't waste a round just reading.
3. **Optimal play** (priority order): Win > Block > Center > Corner > Edge
4. **After you win** (3 in a row), click Verify/Submit immediately in the same round.
5. **After each move**, the opponent may respond. Next round: read updated board + click again.

Key rules:
- 1 round = 1 Evaluate + 1 Click. Never spend a round only gathering data.
- Win lines: [0,1,2], [3,4,5], [6,7,8], [0,3,6], [1,4,7], [2,5,8], [0,4,8], [2,4,6]
- Center=4, Corners=0,2,6,8
"#;

const WORD_SEARCH_SKILL: &str = r#"
Strategy for word search grid challenges:

**GOAL: Extract grid + find ALL words in one Evaluate, then select each word quickly.**

Round 1 - Extract and solve:
1. **Use a single Evaluate to extract grid, find all words, and report positions**:
   ```js
   const cells = [...document.querySelectorAll('[class*=cell], [class*=letter], [class*=grid] > *')];
   const rects = cells.map(c => c.getBoundingClientRect());
   const uniqueTops = [...new Set(rects.map(r => Math.round(r.top)))].sort((a,b) => a-b);
   const rows = uniqueTops.length, cols = Math.round(cells.length / rows);
   const letters = cells.map(c => c.textContent.trim().toUpperCase());
   const grid = []; for (let r = 0; r < rows; r++) grid.push(letters.slice(r*cols,(r+1)*cols));
   const words = [...document.querySelectorAll('[class*=word], [class*=clue], li')].map(el => el.textContent.trim().toUpperCase()).filter(w => w.length > 1);
   const dirs = [[0,1],[0,-1],[1,0],[-1,0],[1,1],[1,-1],[-1,1],[-1,-1]];
   const found = {};
   words.forEach(w => { for(let r=0;r<rows;r++) for(let c=0;c<cols;c++) for(const[dr,dc] of dirs) {
     let ok=true; const idxs=[]; for(let k=0;k<w.length;k++) { const nr=r+dr*k,nc=c+dc*k;
       if(nr<0||nr>=rows||nc<0||nc>=cols||grid[nr][nc]!==w[k]){ok=false;break;} idxs.push(nr*cols+nc); }
     if(ok){found[w]=idxs;return;}
   }});
   document.title = 'FOUND:' + JSON.stringify(found);
   ```
2. **Select and submit each word** using cell indices from the result. For each word:
   ```js
   const cells = [...document.querySelectorAll('[class*=cell], [class*=letter], [class*=grid] > *')];
   [i1, i2, i3, ...].forEach(i => cells[i]?.click());
   ```
   Then click Submit. If drag-selection is needed, use ClickDragPoint from first to last cell.

Key rules:
- Solve the word search algorithmically in JS, don't search visually.
- Words go in 8 directions including diagonal and backwards.
- Select one word at a time, submit, then next word.
"#;

const TEXT_CAPTCHA_SKILL: &str = r##"
Strategy for distorted text / CAPTCHA challenges:

**After 2 failed attempts, STOP guessing and click the refresh button to get new text.**

1. Read the text from the screenshot. Common confusions: O↔D↔0, S↔5, I↔1↔L, Z↔2, B↔8, G↔6
2. Fill the input and submit. Track attempts in memory: `{"op":"set","key":"captcha_attempts","value":1}`
3. If wrong, try ONE alternative reading (swap most ambiguous character). Increment attempts.
4. If 2+ attempts fail, refresh the CAPTCHA:
   ```js
   const r = document.querySelector('[class*=refresh], [class*=reload], button svg');
   if (r) r.click(); else document.querySelectorAll('button').forEach(el => { if (el.innerHTML.includes('refresh') || el.innerHTML.includes('↻')) el.click(); });
   document.title = 'REFRESHED';
   ```
   Or click the ↻ icon near the CAPTCHA image via ClickPoint. Then read the NEW text fresh.
5. Never submit the same text twice.
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
        assert!(registry.len() >= 6);

        // Image grid should match on grid-item class
        let ctx = registry.match_context("", "", "<div class='grid-item'>img</div>");
        assert!(ctx.contains("image-grid-selection"));

        // Tic-tac-toe should match on XOXO title
        let ctx = registry.match_context("", "XOXO Game", "");
        assert!(ctx.contains("tic-tac-toe"));

        // Word search should match on title
        let ctx = registry.match_context("", "Word Search Puzzle", "");
        assert!(ctx.contains("word-search"));

        // Rotation should match on rotate class
        let ctx = registry.match_context("", "", "<div class='rotate-container'>");
        assert!(ctx.contains("rotation-puzzle"));

        // No match on unrelated page
        let ctx = registry.match_context("https://example.com", "Home", "<div>hello</div>");
        assert!(ctx.is_empty());
    }
}
