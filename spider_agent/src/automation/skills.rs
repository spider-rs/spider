//! Dynamic skill loading for web automation.
//!
//! Core types (`Skill`, `SkillTrigger`, `SkillRegistry`) are re-exported from
//! [`spider_skills`]. This module adds spider_agent-specific built-in skills
//! (with `pre_evaluate` JS) and S3 loading.
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

// Re-export core types from spider_skills
pub use spider_skills::{Skill, SkillRegistry, SkillTrigger};

// ─── S3 skill loading types ────────────────────────────────────────────────

/// Configuration for loading skills from an S3-compatible bucket.
#[cfg(feature = "skills_s3")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

// ─── Built-in web challenge skills ──────────────────────────────────────────

/// Create a registry pre-loaded with built-in web challenge skills.
///
/// These skills cover common interactive web challenge patterns:
/// image grid selection, rotation puzzles, tic-tac-toe, word search, etc.
/// Skills include `pre_evaluate` JavaScript that runs before the LLM sees
/// the page, extracting puzzle state automatically.
pub fn builtin_web_challenges() -> SkillRegistry {
    let mut registry = SkillRegistry::new();

    // Image grid selection (e.g., "select all stop signs", "select all vegetables")
    registry.add(
        Skill::new(
            "image-grid-selection",
            "Select matching images from a grid challenge",
        )
        .with_trigger(SkillTrigger::html_contains("grid-item"))
        .with_trigger(SkillTrigger::html_contains("challenge-grid"))
        .with_trigger(SkillTrigger::title_contains("select all"))
        .with_priority(5)
        .with_content(IMAGE_GRID_SKILL),
    );

    // Rotation puzzle — pre_evaluate reads rotation state, model clicks tiles
    registry.add(
        Skill::new(
            "rotation-puzzle",
            "Rotate an image or element to the correct orientation",
        )
        .with_trigger(SkillTrigger::title_contains("rotat"))
        .with_trigger(SkillTrigger::html_contains("rotating-item"))
        .with_priority(5)
        .with_pre_evaluate(ROTATION_PRE_EVALUATE_JS)
        .with_content(ROTATION_SKILL_SIMPLIFIED),
    );

    // Tic-tac-toe / XOXO — high priority to override image-grid when both match
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
            .with_content(TTT_SKILL_SIMPLIFIED),
    );

    // Word search — higher priority than image-grid since word-search pages also have grid-item
    registry.add(
        Skill::new("word-search", "Find and select words in a letter grid")
            .with_trigger(SkillTrigger::title_contains("word search"))
            .with_trigger(SkillTrigger::title_contains("wordsearch"))
            .with_trigger(SkillTrigger::html_contains("word-search-grid-item"))
            .with_trigger(SkillTrigger::html_contains("word-search"))
            .with_priority(8)
            .with_pre_evaluate(WORD_SEARCH_PRE_EVALUATE_JS)
            .with_content(WORD_SEARCH_SKILL_SIMPLIFIED),
    );

    // Text CAPTCHA / math challenges / distorted text
    registry.add(
        Skill::new(
            "text-captcha",
            "Solve text-based CAPTCHAs, distorted text, and math challenges",
        )
        .with_trigger(SkillTrigger::html_contains("captcha-input"))
        .with_trigger(SkillTrigger::html_contains("captcha-text"))
        .with_trigger(SkillTrigger::title_contains("wiggles"))
        .with_priority(3)
        .with_content(TEXT_CAPTCHA_SKILL),
    );

    // Slider / drag challenges
    registry.add(
        Skill::new(
            "slider-drag",
            "Solve slider and drag-to-position challenges",
        )
        .with_trigger(SkillTrigger::html_contains("slider-track"))
        .with_trigger(SkillTrigger::html_contains("slider-handle"))
        .with_trigger(SkillTrigger::html_contains("range-slider"))
        .with_priority(4)
        .with_content(SLIDER_DRAG_SKILL),
    );

    // Checkbox / simple click challenges (L1 of not-a-robot)
    registry.add(
        Skill::new("checkbox-click", "Click a checkbox to prove you are human")
            .with_trigger(SkillTrigger::html_contains("captcha-checkbox"))
            .with_trigger(SkillTrigger::html_contains("checkbox-label"))
            .with_priority(2)
            .with_content(CHECKBOX_SKILL),
    );

    // L8: License plate — read plate from screenshot and type it
    registry.add(
        Skill::new(
            "license-plate",
            "Read a license plate from an image and type it",
        )
        .with_trigger(SkillTrigger::title_contains("license"))
        .with_trigger(SkillTrigger::title_contains("plate"))
        .with_priority(6)
        .with_content(LICENSE_PLATE_SKILL),
    );

    // L9: Nested — recursive stop sign grid subdivision
    registry.add(
        Skill::new(
            "nested-grid",
            "Recursive grid where clicking correct squares subdivides them into smaller ones",
        )
        .with_trigger(SkillTrigger::title_contains("nested"))
        .with_trigger(SkillTrigger::html_contains("nested-container"))
        .with_priority(9) // higher than image-grid to override
        .with_pre_evaluate(NESTED_PRE_EVALUATE_JS)
        .with_content(NESTED_GRID_SKILL),
    );

    // L10: Whack-a-Mole — click moles that pop up
    registry.add(
        Skill::new("whack-a-mole", "Click moles as they pop up in a grid")
            .with_trigger(SkillTrigger::title_contains("whack"))
            .with_trigger(SkillTrigger::title_contains("mole"))
            .with_priority(9)
            .with_pre_evaluate(WHACK_A_MOLE_PRE_EVALUATE_JS)
            .with_content(WHACK_A_MOLE_SKILL),
    );

    // L11: Waldo — find Waldo in a crowded scene
    registry.add(
        Skill::new("find-waldo", "Find Waldo in a crowded Where's Waldo scene")
            .with_trigger(SkillTrigger::title_contains("waldo"))
            .with_priority(9)
            .with_content(FIND_WALDO_SKILL),
    );

    // L12: Muffins? — select chihuahuas, not muffins
    registry.add(
        Skill::new(
            "chihuahua-muffin",
            "Distinguish chihuahuas from muffins in image grid",
        )
        .with_trigger(SkillTrigger::title_contains("muffin"))
        .with_trigger(SkillTrigger::title_contains("chihuahua"))
        .with_priority(9)
        .with_content(CHIHUAHUA_MUFFIN_SKILL),
    );

    // L13: Reverse — select images WITHOUT traffic lights
    registry.add(
        Skill::new(
            "reverse-selection",
            "Select images that do NOT contain the specified object",
        )
        .with_trigger(SkillTrigger::title_contains("reverse"))
        .with_priority(9)
        .with_content(REVERSE_SELECTION_SKILL),
    );

    // L14: Affirmations — find the captcha that says "I'm not a robot"
    registry.add(
        Skill::new(
            "affirmations",
            "Find and select the captcha with specific text",
        )
        .with_trigger(SkillTrigger::title_contains("affirm"))
        .with_priority(6)
        .with_content(AFFIRMATIONS_SKILL),
    );

    // L15: Parking — drag a car into the correct parking spot
    registry.add(
        Skill::new(
            "parking-challenge",
            "Navigate or drag an object into a target zone",
        )
        .with_trigger(SkillTrigger::title_contains("parking"))
        .with_trigger(SkillTrigger::html_contains("parking"))
        .with_priority(6)
        .with_content(PARKING_SKILL),
    );

    // L16: 3D object — identify object shown in 3D perspective
    registry.add(
        Skill::new("3d-object", "Identify objects from 3D perspective")
            .with_trigger(SkillTrigger::title_contains("3d"))
            .with_trigger(SkillTrigger::title_contains("3D"))
            .with_priority(6)
            .with_content(THREE_D_SKILL),
    );

    // L17: Perfect Circle — draw a circle with mouse drag
    registry.add(
        Skill::new(
            "draw-circle",
            "Draw a shape by tracing a mouse path",
        )
        .with_trigger(SkillTrigger::title_contains("circle"))
        .with_trigger(SkillTrigger::title_contains("draw"))
        .with_priority(8)
        .with_pre_evaluate(DRAW_CIRCLE_PRE_EVALUATE_JS)
        .with_content(DRAW_CIRCLE_SKILL),
    );

    // L18: Sisyphus — push a boulder uphill by dragging
    registry.add(
        Skill::new("push-drag", "Repeatedly drag an object in a direction")
            .with_trigger(SkillTrigger::title_contains("sisyphus"))
            .with_trigger(SkillTrigger::html_contains("boulder"))
            .with_priority(6)
            .with_content(PUSH_DRAG_SKILL),
    );

    // L19: In the Dark — find and click elements on a dark screen
    registry.add(
        Skill::new("dark-hidden", "Find elements hidden in darkness")
            .with_trigger(SkillTrigger::title_contains("dark"))
            .with_priority(6)
            .with_content(DARK_HIDDEN_SKILL),
    );

    // L20: Rorschach — describe an inkblot image
    registry.add(
        Skill::new("inkblot-choice", "Interpret or choose from visual prompts")
            .with_trigger(SkillTrigger::title_contains("rorschach"))
            .with_priority(6)
            .with_content(INKBLOT_SKILL),
    );

    // L21: CRAFTCHA — crafting recipe challenge
    registry.add(
        Skill::new("crafting-recipe", "Solve crafting or assembly challenges")
            .with_trigger(SkillTrigger::title_contains("craft"))
            .with_trigger(SkillTrigger::html_contains("craft"))
            .with_priority(6)
            .with_content(CRAFTING_SKILL),
    );

    // L22: My Ducks — arrange or count ducks
    registry.add(
        Skill::new("counting-items", "Count or arrange items in correct order")
            .with_trigger(SkillTrigger::title_contains("duck"))
            .with_priority(6)
            .with_content(COUNTING_SKILL),
    );

    // L23: Panora — match panorama segments
    registry.add(
        Skill::new("panorama-match", "Match or reorder panoramic image segments")
            .with_trigger(SkillTrigger::title_contains("panora"))
            .with_priority(6)
            .with_content(PANORAMA_SKILL),
    );

    // L24: Eye Exam — read text from an eye chart
    registry.add(
        Skill::new("eye-chart", "Read text from decreasing-size visual display")
            .with_trigger(SkillTrigger::title_contains("eye"))
            .with_trigger(SkillTrigger::title_contains("exam"))
            .with_priority(6)
            .with_content(EYE_CHART_SKILL),
    );

    // L25: Creativity — draw or create something
    registry.add(
        Skill::new("creative-draw", "Draw or create something original")
            .with_trigger(SkillTrigger::title_contains("creativ"))
            .with_priority(6)
            .with_content(CREATIVE_SKILL),
    );

    // L27: Networking — connect nodes or people
    registry.add(
        Skill::new("network-connect", "Connect nodes or items together")
            .with_trigger(SkillTrigger::title_contains("network"))
            .with_priority(6)
            .with_content(NETWORK_SKILL),
    );

    // L28: Day Trader — buy/sell timing challenge
    registry.add(
        Skill::new("trading-timing", "Buy and sell at the right time")
            .with_trigger(SkillTrigger::title_contains("trader"))
            .with_trigger(SkillTrigger::title_contains("trading"))
            .with_priority(6)
            .with_content(TRADING_SKILL),
    );

    // L29: Soul — philosophical choice
    registry.add(
        Skill::new("text-choice", "Make a text-based choice or response")
            .with_trigger(SkillTrigger::title_contains("soul"))
            .with_priority(5)
            .with_content(TEXT_CHOICE_SKILL),
    );

    // L30: Sliding Tiles — sliding puzzle solver
    registry.add(
        Skill::new("sliding-puzzle", "Solve a sliding tile puzzle")
            .with_trigger(SkillTrigger::title_contains("sliding"))
            .with_trigger(SkillTrigger::html_contains("sliding"))
            .with_trigger(SkillTrigger::html_contains("puzzle-grid"))
            .with_priority(8)
            .with_pre_evaluate(SLIDING_PUZZLE_PRE_EVALUATE_JS)
            .with_content(SLIDING_PUZZLE_SKILL),
    );

    // L31: Traffic Tree — traffic signal tree challenge
    registry.add(
        Skill::new("traffic-signal", "Interact with traffic signals or trees")
            .with_trigger(SkillTrigger::title_contains("traffic"))
            .with_priority(6)
            .with_content(TRAFFIC_SIGNAL_SKILL),
    );

    // L32: Drum Verify — rhythm/drum pattern
    registry.add(
        Skill::new("rhythm-pattern", "Reproduce a rhythm or sound pattern")
            .with_trigger(SkillTrigger::title_contains("drum"))
            .with_trigger(SkillTrigger::title_contains("rhythm"))
            .with_priority(7)
            .with_content(RHYTHM_SKILL),
    );

    // L33: Brands — identify brand logos
    registry.add(
        Skill::new("brand-logo", "Identify brand logos from images")
            .with_trigger(SkillTrigger::title_contains("brand"))
            .with_priority(6)
            .with_content(BRAND_LOGO_SKILL),
    );

    // L34: Mathematics — solve math equations
    registry.add(
        Skill::new("math-solver", "Solve mathematical equations or expressions")
            .with_trigger(SkillTrigger::title_contains("math"))
            .with_trigger(SkillTrigger::title_contains("equation"))
            .with_priority(7)
            .with_pre_evaluate(MATH_PRE_EVALUATE_JS)
            .with_content(MATH_SKILL),
    );

    // L35: Shuffle — track card during shuffle
    registry.add(
        Skill::new("card-tracking", "Track an object through movement or shuffle")
            .with_trigger(SkillTrigger::title_contains("shuffle"))
            .with_priority(7)
            .with_content(SHUFFLE_SKILL),
    );

    // L36: Not Candy Crush — match-3 game
    registry.add(
        Skill::new("match3-game", "Solve a match-3 or tile-matching puzzle")
            .with_trigger(SkillTrigger::title_contains("candy"))
            .with_trigger(SkillTrigger::title_contains("crush"))
            .with_trigger(SkillTrigger::title_contains("match"))
            .with_priority(7)
            .with_content(MATCH3_SKILL),
    );

    // L37: Imposters — find the odd one out
    registry.add(
        Skill::new("odd-one-out", "Find the imposter or odd item among similar ones")
            .with_trigger(SkillTrigger::title_contains("imposter"))
            .with_trigger(SkillTrigger::title_contains("impostor"))
            .with_priority(7)
            .with_content(ODD_ONE_OUT_SKILL),
    );

    // L38: Tough Decisions — make a choice between options
    registry.add(
        Skill::new("decision-choice", "Make a decision between presented options")
            .with_trigger(SkillTrigger::title_contains("decision"))
            .with_trigger(SkillTrigger::title_contains("choose"))
            .with_priority(5)
            .with_content(DECISION_SKILL),
    );

    // L39: Facial Exam — face recognition or matching
    registry.add(
        Skill::new("face-matching", "Match or compare faces")
            .with_trigger(SkillTrigger::title_contains("facial"))
            .with_trigger(SkillTrigger::title_contains("face"))
            .with_priority(6)
            .with_content(FACE_MATCHING_SKILL),
    );

    // L40: Slot Machine — stop slots at the right time
    registry.add(
        Skill::new("slot-machine", "Stop a spinning element at the right moment")
            .with_trigger(SkillTrigger::title_contains("slot"))
            .with_priority(6)
            .with_content(SLOT_MACHINE_SKILL),
    );

    // L41: Grave — dig or find hidden item
    registry.add(
        Skill::new("dig-find", "Dig or search for a hidden element")
            .with_trigger(SkillTrigger::title_contains("grave"))
            .with_priority(6)
            .with_content(DIG_FIND_SKILL),
    );

    // L42: Reverse Turing — convince you're human via text
    registry.add(
        Skill::new("turing-text", "Type a convincing human-like text response")
            .with_trigger(SkillTrigger::title_contains("turing"))
            .with_priority(7)
            .with_content(TURING_TEXT_SKILL),
    );

    // L43: Ikea — identify or assemble furniture
    registry.add(
        Skill::new("assembly-id", "Identify items from assembly instructions")
            .with_trigger(SkillTrigger::title_contains("ikea"))
            .with_trigger(SkillTrigger::title_contains("assembl"))
            .with_priority(6)
            .with_content(ASSEMBLY_SKILL),
    );

    // L44: Grandmaster — chess challenge
    registry.add(
        Skill::new("chess-challenge", "Make the best chess move")
            .with_trigger(SkillTrigger::title_contains("chess"))
            .with_trigger(SkillTrigger::title_contains("grandmaster"))
            .with_trigger(SkillTrigger::html_contains("chess"))
            .with_priority(8)
            .with_content(CHESS_SKILL),
    );

    // L45: Jessica — find a specific person
    registry.add(
        Skill::new("find-person", "Find a specific person in a crowd or grid")
            .with_trigger(SkillTrigger::title_contains("jessica"))
            .with_priority(6)
            .with_content(FIND_PERSON_SKILL),
    );

    // L46: Floors — navigate building floors
    registry.add(
        Skill::new("floor-nav", "Navigate through building floors")
            .with_trigger(SkillTrigger::title_contains("floor"))
            .with_priority(6)
            .with_content(FLOOR_NAV_SKILL),
    );

    // L47: Din Don Dan — bell or sound pattern
    registry.add(
        Skill::new("bell-pattern", "Reproduce a bell or sound sequence")
            .with_trigger(SkillTrigger::title_contains("din"))
            .with_trigger(SkillTrigger::title_contains("don"))
            .with_trigger(SkillTrigger::title_contains("dan"))
            .with_priority(7)
            .with_content(BELL_PATTERN_SKILL),
    );

    // L48: The Inventor — final creative challenge
    registry.add(
        Skill::new("final-creative", "Complete a creative final challenge")
            .with_trigger(SkillTrigger::title_contains("inventor"))
            .with_priority(6)
            .with_content(FINAL_CREATIVE_SKILL),
    );

    registry
}

// ─── S3 skill loading ───────────────────────────────────────────────────────

/// Load skills from an S3-compatible bucket into an existing registry.
///
/// Name conflicts: S3 skills replace any existing skill with the same name.
/// Returns the count of successfully loaded skills.
#[cfg(feature = "skills_s3")]
pub async fn load_from_s3(
    registry: &mut SkillRegistry,
    source: &S3SkillSource,
) -> Result<usize, S3SkillError> {
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
                let matches_ext = exts.iter().any(|ext| key.ends_with(&format!(".{}", ext)));
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
                    serde_json::from_str::<Skill>(&text).map_err(|e| S3SkillError::ParseError {
                        key: key.to_string(),
                        reason: e.to_string(),
                    })?
                } else {
                    // Markdown
                    Skill::from_markdown(&text).ok_or_else(|| S3SkillError::ParseError {
                        key: key.to_string(),
                        reason: "invalid markdown frontmatter".to_string(),
                    })?
                };

                // Replace existing skill with same name
                registry.remove(&skill.name);
                registry.add(skill);
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
#[cfg(feature = "skills_s3")]
pub async fn with_builtin_and_s3(source: &S3SkillSource) -> Result<SkillRegistry, S3SkillError> {
    let mut registry = builtin_web_challenges();
    load_from_s3(&mut registry, source).await?;
    Ok(registry)
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
/// Strategy: win → block → fork (create 2+ threats) → block fork → center → opposite corner → corner → side.
const TTT_PRE_EVALUATE_JS: &str = r##"try{const cells=[...document.querySelectorAll('.grid-item')].filter(el=>el.offsetWidth>20&&el.offsetHeight>20);const n=cells.length;const occ=new Set();cells.forEach((el,i)=>{const inner=el.querySelector('.tic-tac-toe-cell');if(inner&&(inner.className.includes('cell-selected')||inner.className.includes('cell-disabled')))occ.add(i);});let tr=document.getElementById('ttt-h');if(!tr){tr=document.createElement('div');tr.id='ttt-h';tr.style.display='none';tr.dataset.m='';document.body.appendChild(tr);}const my=new Set(tr.dataset.m?tr.dataset.m.split(',').map(Number):[]);if(occ.size===0)my.clear();for(const m of my)if(!occ.has(m))my.delete(m);const opp=new Set([...occ].filter(i=>!my.has(i)));const board=Array.from({length:9},(_,i)=>my.has(i)?'M':opp.has(i)?'T':'.');const W=[[0,1,2],[3,4,5],[6,7,8],[0,3,6],[1,4,7],[2,5,8],[0,4,8],[2,4,6]];let myWin=false,thWin=false;for(const w of W){if(w.every(i=>board[i]==='M'))myWin=true;if(w.every(i=>board[i]==='T'))thWin=true;}
function countThreats(b,mark){let t=0;for(const w of W){const m=w.filter(i=>b[i]===mark),e=w.filter(i=>b[i]==='.');if(m.length===2&&e.length===1)t++;}return t;}
let best=-1;if(!myWin&&!thWin){
for(const w of W){const mi=w.filter(i=>board[i]==='M'),e=w.filter(i=>board[i]==='.');if(mi.length===2&&e.length===1){best=e[0];break;}}
if(best<0)for(const w of W){const ti=w.filter(i=>board[i]==='T'),e=w.filter(i=>board[i]==='.');if(ti.length===2&&e.length===1){best=e[0];break;}}
if(best<0){let bestFork=-1,bestScore=0;for(let i=0;i<9;i++){if(board[i]!=='.')continue;const b2=[...board];b2[i]='M';const threats=countThreats(b2,'M');if(threats>=2&&threats>bestScore){bestScore=threats;bestFork=i;}}best=bestFork;}
if(best<0){for(let i=0;i<9;i++){if(board[i]!=='.')continue;const b2=[...board];b2[i]='T';if(countThreats(b2,'T')>=2){best=i;break;}}}
if(best<0&&board[4]==='.')best=4;
if(best<0){const opc=[[0,8],[2,6],[6,2],[8,0]];for(const[c,o]of opc)if(board[c]==='T'&&board[o]==='.')best=o;}
if(best<0)for(const c of[0,2,6,8])if(board[c]==='.'){best=c;break;}
if(best<0)for(const c of[1,3,5,7])if(board[c]==='.'){best=c;break;}
}let clicked=false;if(best>=0&&cells[best]){const el=cells[best];const r=el.getBoundingClientRect();const ev={bubbles:true,cancelable:true,clientX:r.x+r.width/2,clientY:r.y+r.height/2};el.dispatchEvent(new PointerEvent('pointerdown',ev));el.dispatchEvent(new MouseEvent('mousedown',ev));el.dispatchEvent(new PointerEvent('pointerup',ev));el.dispatchEvent(new MouseEvent('mouseup',ev));el.dispatchEvent(new MouseEvent('click',ev));clicked=true;my.add(best);board[best]='M';for(const w of W)if(w.every(i=>board[i]==='M'))myWin=true;}tr.dataset.m=[...my].join(',');const full=!board.includes('.');document.title='TTT:'+JSON.stringify({n,board:board.join(''),best,clicked,myWin,thWin,full});}catch(e){document.title='TTT_ERR:'+e.message;}
"##;

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
/// Extracts grid, finds words, outputs coordinates for engine-side CDP drag.
const WORD_SEARCH_PRE_EVALUATE_JS: &str = r##"try{
const doneEl=document.getElementById('ws-engine-done');
if(doneEl){document.title=doneEl.dataset.t;throw'done';}
let cells=[...document.querySelectorAll('.word-search-grid-item')];
if(!cells.length)cells=[...document.querySelectorAll('.grid-item')].filter(el=>el.textContent.trim().length===1);
if(!cells.length)cells=[...document.querySelectorAll('[class*=letter]')].filter(el=>el.textContent.trim().length===1);
const n=cells.length;
if(n<4){document.title='WS:'+JSON.stringify({n,err:'no_grid'});}
else{
const rects=cells.map(c=>{const r=c.getBoundingClientRect();return{x:Math.round(r.x+r.width/2),y:Math.round(r.y+r.height/2)};});
const letters=cells.map(c=>c.textContent.trim().toUpperCase());
const tops=[...new Set(rects.map(r=>r.y))].sort((a,b)=>a-b);
const rows=tops.length||1,cols=Math.round(n/rows)||1;
const grid=[];for(let r=0;r<rows;r++)grid.push(letters.slice(r*cols,(r+1)*cols));
const dirs=[[0,1],[0,-1],[1,0],[-1,0],[1,1],[1,-1],[-1,1],[-1,-1]];
function findWord(w){for(let r=0;r<rows;r++)for(let c=0;c<cols;c++)for(const[dr,dc]of dirs){let ok=true;for(let k=0;k<w.length;k++){const nr=r+dr*k,nc=c+dc*k;if(nr<0||nr>=rows||nc<0||nc>=cols||grid[nr][nc]!==w[k]){ok=false;break;}}if(ok){const pts=[];for(let k=0;k<w.length;k++){const idx=(r+dr*k)*cols+(c+dc*k);pts.push(rects[idx]);}return pts;}}return null;}
let words=[];
let wordEls=[...document.querySelectorAll('.word-search-words span,.word-search-word')];
if(!wordEls.length)wordEls=[...document.querySelectorAll('[class*=word-item],[class*=clue]')];
if(!wordEls.length)wordEls=[...document.querySelectorAll('.word-search-words li,.words li')];
if(!wordEls.length)wordEls=[...document.querySelectorAll('.words span')];
words=wordEls.map(el=>el.textContent.trim().toUpperCase().replace(/\s+/g,'')).filter(w=>w.length>1&&w.length<=20&&w.match(/^[A-Z]+$/));
if(!words.length){
  const txts=[...document.querySelectorAll('h1,h2,h3,h4,p,span,div,label')];
  for(const el of txts){
    const t=el.textContent.trim();
    if(t.length>200||t.length<8)continue;
    const m=t.match(/(?:select|find|choose|highlight)\s+(?:all\s+)?(?:the\s+)?(?:squares?\s+)?(?:with|containing|that\s+have|showing|of)\s+(?:a\s+|an?\s+)?(.+)/i);
    if(m){
      const raw=m[1].replace(/[.!?]+$/,'').split(/\s+and\s+|,\s*/i).map(s=>s.trim());
      words=raw.map(s=>s.toUpperCase().replace(/[^A-Z]/g,'')).filter(w=>w.length>=2);
      if(words.length)break;
    }
  }
}
const found={};
words.forEach(w=>{
  const pts=findWord(w);
  if(pts){found[w]=pts;return;}
  for(let i=2;i<=w.length-2;i++){const p1=w.slice(0,i),p2=w.slice(i);const r1=findWord(p1),r2=findWord(p2);if(r1&&r2){found[p1]=r1;found[p2]=r2;return;}}
});
const fk=Object.keys(found);
if(fk.length>0){document.title='WS_DRAG:'+JSON.stringify({n,words:fk,drags:fk.map(w=>found[w])});}
else{const gt=grid.map(r=>r.join('')).join('/');document.title='WS:'+JSON.stringify({n,rows,cols,grid:gt,words,found:{}});}
}}catch(e){if(e!=='done'){document.title='WS_ERR:'+(e&&e.message||String(e));}}
"##;

/// Simplified Word Search skill content — engine handles CDP drag, model clicks verify.
const WORD_SEARCH_SKILL_SIMPLIFIED: &str = r##"
Word search puzzle: Words are auto-found and auto-dragged by the engine. Read `document.title`.

**(Skip image-grid-selection skill — this is a word search, NOT an image grid.)**

Title formats:
- `WS_DONE:{"dragged":["STOPSIGN","BIKE"]}` — engine dragged all words. Click verify!
- `WS:{"n":100,...,"words":[],"found":{}}` — grid found but words not detected.
- `WS_ERR:...` or `WS:{"n":0,...}` — grid not loaded.

**Rules (check title EVERY round):**
- **WS_DONE** → words selected by engine! Click verify: `[{"Click":"#captcha-verify-button"}]`
- **WS: with found empty** → words not detected. Use Evaluate to read page instruction text.
- **WS_ERR or n is 0** → grid not loaded: `[{"Wait":1000}]`
- After verify, if still on word search, refresh: `[{"Click":".captcha-refresh"},{"Wait":800}]`

**Do NOT write any drag/click JS. Word selection is automatic.**
"##;

const TEXT_CAPTCHA_SKILL: &str = r##"
Distorted text CAPTCHA: Read the wiggling/wavy text from the SCREENSHOT and type it.

**CRITICAL RULES:**
- DO NOT type the level/challenge name (e.g., "HUMAN", "WIGGLES", "CAPTCHA").
- The answer is 4-8 random uppercase letters in a wavy/distorted style. They are RANDOM, not English words.
- DO NOT analyze canvas pixels or write JS. Just READ the distorted text visually.
- Never submit the same text twice. Check session memory for previous attempts.

**Steps (solve in 1 round):**
```json
"steps": [
  {"Clear":".captcha-input-text"},
  {"Fill":{"selector":".captcha-input-text","value":"YOURTEXT"}},
  {"Click":".captcha-button"}
]
```

If wrong after 1 try: swap ambiguous chars (O↔D↔0, S↔5, I↔1↔L, Z↔2, B↔8).
**After 2 fails → refresh captcha using one of these (try in order):**
1. `[{"Click":"img.captcha-refresh"},{"Wait":1500}]`
2. `[{"Click":"[class*=refresh]"},{"Wait":1500}]`
3. Use ClickPoint on the small refresh/reload icon visible in the screenshot.
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

// ─── L8-L14 skill content ────────────────────────────────────────────────

const LICENSE_PLATE_SKILL: &str = r##"
License plate challenge: Read the license plate from the car image and type it exactly.

**CRITICAL RULES:**
- Look at the screenshot carefully for the license plate on the back of the car.
- Type the plate EXACTLY as shown — include spaces, dashes, and correct capitalization.
- Letters are uppercase. Include any state/country text only if the input field expects it.

**Steps (solve in 1 round):**
```json
"steps": [
  {"Clear":".captcha-input-text"},
  {"Fill":{"selector":".captcha-input-text","value":"ABC 1234"}},
  {"Click":"#captcha-verify-button"}
]
```

If wrong after 1 try: re-read the plate carefully — common confusions: 0↔O, 1↔I↔L, 8↔B, 5↔S.
**After 2 fails → refresh:** `[{"Click":".captcha-refresh"},{"Wait":1000}]`
"##;

/// JS executed by the engine before the LLM sees the nested grid page.
/// Detects stop sign image, computes bounding-rect overlap with leaf boxes,
/// and reports JSON with toClick array for engine auto-clicking.
const NESTED_PRE_EVALUATE_JS: &str = r##"try{
var done=document.getElementById('nest-engine-done');
if(done){document.title=done.dataset.t;throw'done';}
var all=[...document.querySelectorAll('.box')];
var leaf=all.filter(function(b){return !b.querySelector('.box')&&!b.querySelector('.nested-container');});
leaf.forEach(function(b,i){b.setAttribute('data-spider-id',String(i));});
var ssRect=null;
var imgs=[...document.querySelectorAll('img')].filter(function(el){var r=el.getBoundingClientRect();return r.width>20&&r.height>20;});
if(imgs.length>0){var best=imgs[0];for(var im of imgs){var rr=im.getBoundingClientRect();if(rr.width*rr.height>best.getBoundingClientRect().width*best.getBoundingClientRect().height)best=im;}ssRect=best.getBoundingClientRect();}
if(!ssRect){var cands=[...document.querySelectorAll('[class*=sign],[class*=stop],[class*=image],[class*=captcha-image]')].filter(function(el){var r=el.getBoundingClientRect();return r.width>20&&r.height>20;});if(cands.length>0)ssRect=cands[0].getBoundingClientRect();}
if(!ssRect){var bgEls=[...document.querySelectorAll('.nested-container,div')].filter(function(el){var bg=getComputedStyle(el).backgroundImage;return bg&&bg!=='none';});if(bgEls.length>0)ssRect=bgEls[0].getBoundingClientRect();}
var toClick=[];var selected=[];
var boxes=leaf.map(function(b,i){var r=b.getBoundingClientRect();var sel=b.classList.contains('selected');if(sel){selected.push(i);}else if(ssRect){var ox=Math.max(0,Math.min(r.right,ssRect.right)-Math.max(r.left,ssRect.left));var oy=Math.max(0,Math.min(r.bottom,ssRect.bottom)-Math.max(r.top,ssRect.top));if(ox>2&&oy>2)toClick.push(i);}
return{id:i,x:Math.round(r.x+r.width/2),y:Math.round(r.y+r.height/2),w:Math.round(r.width),h:Math.round(r.height)};});
document.title='NEST:'+JSON.stringify({total:leaf.length,sel:selected.length,toClick:toClick,selected:selected,hasSign:!!ssRect,boxes:boxes});
}catch(e){if(e!=='done')document.title='NEST_ERR:'+e.message;}
"##;

const NESTED_GRID_SKILL: &str = r##"
Nested grid: "Select all squares with a stop sign" — squares SUBDIVIDE when clicked correctly.

**Engine auto-solves this level.** Pre-evaluate detects stop sign overlap and clicks boxes automatically.
Title format: `NEST:{"total":N,"sel":N,"toClick":[ids],"hasSign":true,"boxes":[...]}`

If engine already solved (title starts with NEST_DONE), just click verify:
`[{"Click":"#captcha-verify-button"}]`

If engine missed boxes (verify failed), look at screenshot for any unselected boxes overlapping the stop sign.
Click them with `[data-spider-id='N']` selectors, then re-verify.
"##;

/// JS executed by the engine for whack-a-mole — detects and clicks visible moles.
const WHACK_A_MOLE_PRE_EVALUATE_JS: &str = r##"try{
var done=document.getElementById('wam-engine-done');
if(done){document.title=done.dataset.t;throw'done';}
var moles=[...document.querySelectorAll('.grid-item,.mole,.whack-target,[class*=mole]')].filter(el=>{
  var r=el.getBoundingClientRect();
  if(r.width<10||r.height<10)return false;
  var s=getComputedStyle(el);
  if(s.display==='none'||s.visibility==='hidden'||s.opacity==='0')return false;
  var img=el.querySelector('img');
  if(img){var src=img.src||img.dataset.src||'';if(src.includes('mole')||src.includes('gopher'))return true;}
  var bg=s.backgroundImage||'';
  if(bg.includes('mole')||bg.includes('gopher'))return true;
  var cls=(el.className||'').toLowerCase();
  if(cls.includes('mole')||cls.includes('active')||cls.includes('visible'))return true;
  return false;
});
var clicked=0;
moles.forEach(function(el){
  var r=el.getBoundingClientRect();
  var ev={bubbles:true,cancelable:true,clientX:r.x+r.width/2,clientY:r.y+r.height/2};
  el.dispatchEvent(new PointerEvent('pointerdown',ev));
  el.dispatchEvent(new MouseEvent('mousedown',ev));
  el.dispatchEvent(new PointerEvent('pointerup',ev));
  el.dispatchEvent(new MouseEvent('mouseup',ev));
  el.dispatchEvent(new MouseEvent('click',ev));
  clicked++;
});
document.title='WAM:'+JSON.stringify({moles:moles.length,clicked:clicked});
}catch(e){if(e!=='done')document.title='WAM_ERR:'+(e&&e.message||String(e));}
"##;

const WHACK_A_MOLE_SKILL: &str = r##"
Whack-a-Mole: Click moles as they pop up. Hit 5 moles to pass.

**Auto-detection runs via pre_evaluate.** Read `document.title` for state.

Title format: `WAM:{"moles":N,"clicked":N}`

**Rules (check title EVERY round):**
- **clicked > 0** → moles were auto-clicked! Wait for more to appear: `[{"Wait":800}]`
- **moles is 0** → no moles visible yet. Wait: `[{"Wait":500}]`
- **WAM_ERR** → detection failed. Use screenshot to find moles visually.

**If auto-detection misses moles (you see them in screenshot):**
- Use ClickPoint on each visible mole: `[{"ClickPoint":{"x":300,"y":400}},{"Wait":300}]`
- Moles pop up briefly — click fast, don't wait between clicks.
- You need to hit 5 total. If you accidentally click grass, it may deselect.

**After 5 hits, click verify:** `[{"Click":"#captcha-verify-button"}]`
"##;

const FIND_WALDO_SKILL: &str = r##"
Where's Waldo: Find Waldo in a crowded beach scene grid.

**Waldo's appearance:**
- Tall man with dark brown hair and black glasses
- Red and white horizontally STRIPED shirt (most distinctive feature)
- Blue jeans
- Red and white striped beanie/hat
- Often partially hidden behind other characters

**STRATEGY:**
1. Scan the screenshot carefully for red-and-white stripes.
2. Waldo is typically in the upper-right area of the image.
3. Once found, click the grid square(s) containing Waldo.
4. He spans 2 vertical squares — select BOTH (head square + body square).

**Steps:**
```json
"steps": [
  {"ClickPoint":{"x":WALDOx,"y":WALDOy_HEAD}},
  {"ClickPoint":{"x":WALDOx,"y":WALDOy_BODY}},
  {"Wait":500},
  {"Click":"#captcha-verify-button"}
]
```

**Tips:**
- Look for the RED AND WHITE STRIPES pattern — it's the most visible feature.
- Don't confuse with other striped items (umbrellas, towels). Waldo is a PERSON.
- If verify fails, you may have missed a square. Check for his hat above.
- After 2 fails → refresh: `[{"Click":".captcha-refresh"},{"Wait":1000}]`
"##;

const CHIHUAHUA_MUFFIN_SKILL: &str = r##"
Muffins? challenge: Select all CHIHUAHUAS — not the muffins!

**The classic chihuahua vs muffin visual trick.** Images of chihuahuas and blueberry muffins look very similar.

**How to tell them apart:**
- **Chihuahuas**: Have EYES (shiny, reflective), a NOSE (small dark triangle), EARS (pointed, stand up), fur texture varies
- **Muffins**: Have a WRAPPER/PAPER cup at bottom, more uniform round dome shape, visible blueberries/chocolate chips as dark spots, crumbly top texture

**Key differences:**
- Chihuahuas have a visible snout/mouth area; muffins have a flat dome
- Chihuahuas' eyes are positioned symmetrically and REFLECT light; muffin spots don't
- Muffin wrappers have ridged edges at the base
- Chihuahua ears are triangular and stick up; muffins have no pointy features

**Steps:**
```json
"steps": [
  {"Click":".grid-item:nth-child(N)"},
  ... (all chihuahua tiles)
  {"Wait":300},
  {"Click":"#captcha-verify-button"}
]
```

**SOLVE IN 2 ROUNDS MAX.** If verify fails, toggle your selections and retry.
After 2 fails → refresh: `[{"Click":".captcha-refresh"},{"Wait":1000}]`
"##;

const REVERSE_SELECTION_SKILL: &str = r##"
Reverse selection: Select all images that do NOT contain the specified object.

**THIS IS THE OPPOSITE of normal selection.** The title says "select images WITHOUT [object]".

**CRITICAL:** You must select tiles that DO NOT have the object. Leave tiles WITH the object unselected.

**Strategy:**
1. Read the instruction carefully — note what object to AVOID.
2. Look at each grid tile in the screenshot.
3. Click tiles that do NOT contain the specified object.
4. Leave tiles containing the object UNCLICKED.

**Common object: traffic lights**
- Tiles WITH traffic lights → do NOT click
- Tiles WITHOUT traffic lights → DO click

**Steps:**
```json
"steps": [
  {"Click":".grid-item:nth-child(1)"},
  {"Click":".grid-item:nth-child(3)"},
  {"Click":".grid-item:nth-child(5)"},
  ... (all tiles WITHOUT the object)
  {"Wait":300},
  {"Click":"#captcha-verify-button"}
]
```

**SOLVE IN 2 ROUNDS MAX.** After 2 fails → refresh: `[{"Click":".captcha-refresh"},{"Wait":1000}]`
"##;

const AFFIRMATIONS_SKILL: &str = r##"
Affirmations: Find and click the captcha text that says "I'm not a robot" (or similar).

**Multiple text options are displayed.** Only ONE says the right affirmation.

**Strategy:**
1. Read ALL visible text options in the screenshot carefully.
2. Find the one that says "I'm not a robot" (exact or very close match).
3. Click on that text element.
4. Then click verify.

**Steps:**
```json
"steps": [
  {"ClickPoint":{"x":CAPTCHAx,"y":CAPTCHAy}},
  {"Wait":300},
  {"Click":"#captcha-verify-button"}
]
```

If there are multiple similar texts, look for the EXACT phrase "I'm not a robot".
Solve in 1-2 rounds.
"##;

// ─── L15+ skill constants ─────────────────────────────────────────────────

const PARKING_SKILL: &str = r##"
Parking challenge: Drag or steer an object into the highlighted target zone.

**Strategy:**
1. Identify the car/object and the target parking spot in the screenshot.
2. Use ClickDragPoint to drag the car from its current position INTO the parking spot.
3. Or use arrow key presses if steering controls are present.

**Drag approach:**
```json
"steps": [
  {"ClickDragPoint":{"startX":CARx,"startY":CARy,"endX":SPOTx,"endY":SPOTy}},
  {"Wait":500},
  {"Click":"#captcha-verify-button"}
]
```

**Arrow key approach:**
```json
"steps": [
  {"KeyDown":"ArrowUp"},{"Wait":200},{"KeyDown":"ArrowUp"},{"Wait":200},
  {"KeyDown":"ArrowLeft"},{"Wait":200},
  {"Click":"#captcha-verify-button"}
]
```

Look at the layout carefully. If there are steering wheel controls, click them. Solve in 3-5 rounds.
"##;

const THREE_D_SKILL: &str = r##"
3D object challenge: Identify what the object is when shown in 3D perspective.

**Strategy:**
1. Look at the 3D-rendered object in the screenshot.
2. Identify what it represents (common objects: chair, cup, car, shoe, etc.).
3. Type the answer or click the matching option.

If there's a text input, use Fill to type the object name.
If there are clickable options, click the correct one.
Solve in 2-3 rounds.
"##;

/// JS that computes a circular mouse path and reports center/radius for engine drag.
const DRAW_CIRCLE_PRE_EVALUATE_JS: &str = r##"try{
var done=document.getElementById('circle-engine-done');
if(done){document.title=done.dataset.t;throw'done';}
var canvas=document.querySelector('canvas');
if(!canvas){var divs=[...document.querySelectorAll('div')].filter(function(d){var r=d.getBoundingClientRect();return r.width>100&&r.height>100&&r.width<800;});canvas=divs[0];}
if(canvas){var r=canvas.getBoundingClientRect();var cx=Math.round(r.x+r.width/2);var cy=Math.round(r.y+r.height/2);var rad=Math.round(Math.min(r.width,r.height)*0.35);var pts=[];for(var i=0;i<=36;i++){var a=i*Math.PI*2/36;pts.push({x:Math.round(cx+rad*Math.cos(a)),y:Math.round(cy+rad*Math.sin(a))});}
document.title='CIRCLE:'+JSON.stringify({cx:cx,cy:cy,rad:rad,pts:pts});}
else{document.title='CIRCLE_ERR:no_canvas';}
}catch(e){if(e!=='done')document.title='CIRCLE_ERR:'+e.message;}
"##;

const DRAW_CIRCLE_SKILL: &str = r##"
Draw circle: Trace a circular path with the mouse.

**Engine auto-draws when CIRCLE: title is set.** Pre-evaluate computes 36-point circle path.
Title format: `CIRCLE:{"cx":N,"cy":N,"rad":N,"pts":[{x,y}...]}`

If engine drew (CIRCLE_DONE), click verify.
If engine couldn't find canvas, manually use ClickDragPoint to trace a rough circle.

**Manual fallback:** estimate center and radius from screenshot, then drag in a loop.
"##;

const PUSH_DRAG_SKILL: &str = r##"
Push/drag challenge: Continuously drag an object in a direction.

**Strategy:**
1. Find the draggable object (boulder, ball, etc.) in the screenshot.
2. Drag it in the required direction — usually uphill or towards a target.
3. Repeat the drag multiple times since the object may slide back.

**Steps:**
```json
"steps": [
  {"ClickDragPoint":{"startX":OBJx,"startY":OBJy,"endX":TARGETx,"endY":TARGETy}},
  {"Wait":300},
  {"ClickDragPoint":{"startX":OBJx2,"startY":OBJy2,"endX":TARGETx,"endY":TARGETy}},
  {"Wait":300}
]
```

Repeat dragging 3-5 times per round. Object position changes after each drag.
After reaching the target, click verify. Solve in 3-6 rounds.
"##;

const DARK_HIDDEN_SKILL: &str = r##"
Dark/hidden challenge: Find elements on a very dark or hidden screen.

**Strategy:**
1. The screen appears dark — look VERY carefully at the screenshot.
2. There may be a faint outline, subtle glow, or slightly different shade.
3. Move the mouse around to reveal hidden elements (some respond to hover).
4. Use Evaluate to check for interactive elements: `document.querySelectorAll('[class*=hidden],[style*=opacity],[class*=dark]')`.
5. Try clicking in the center area or where you see subtle differences.

**Reveal approach:**
```json
"steps": [
  {"ClickPoint":{"x":400,"y":300}},
  {"Wait":500},
  {"ClickPoint":{"x":600,"y":400}},
  {"Wait":500}
]
```

If nothing visible, try clicking around the center systematically.
"##;

const INKBLOT_SKILL: &str = r##"
Inkblot/interpretation challenge: Choose what you see in an abstract image.

**Strategy:**
1. Look at the inkblot or abstract image in the screenshot.
2. Read the available choice options.
3. Pick the most common/obvious interpretation (butterfly, bat, moth, face are common Rorschach answers).
4. Click the matching option.

If there's a text input, type a common Rorschach answer.
If there are clickable buttons/options, click the best match.
Solve in 1-2 rounds.
"##;

const CRAFTING_SKILL: &str = r##"
Crafting/assembly challenge: Combine items to create something.

**Strategy:**
1. Look at the crafting grid and available materials in the screenshot.
2. Identify what needs to be crafted (check title or instructions).
3. Drag items from inventory into the correct grid positions.
4. For Minecraft-style: place items in the 3x3 grid pattern.

**Common recipes:**
- Stick: 2 planks vertically
- Planks: 1 log anywhere
- Torch: stick below, coal above
- Sword: stick below, 2 material above

Drag items using ClickDragPoint from source to grid cell. Solve in 2-4 rounds.
"##;

const COUNTING_SKILL: &str = r##"
Counting/arrangement challenge: Count items or arrange them correctly.

**Strategy:**
1. Count the items in the screenshot carefully.
2. Type the number or arrange items by dragging.
3. If arranging: drag items to their correct positions using ClickDragPoint.
4. If counting: enter the count via Fill in the input field.

Double-check your count before submitting. Solve in 1-3 rounds.
"##;

const PANORAMA_SKILL: &str = r##"
Panorama matching: Reorder or match panoramic image segments.

**Strategy:**
1. Look at the image segments in the screenshot.
2. Identify the correct order that forms a continuous panoramic view.
3. Drag segments into the correct positions using ClickDragPoint.
4. Match edges — adjacent segments should have continuous lines/colors.

Look for horizon lines, buildings, roads that continue across segments.
Solve in 2-4 rounds.
"##;

const EYE_CHART_SKILL: &str = r##"
Eye chart challenge: Read text from a vision-test style display.

**Strategy:**
1. Look at the eye chart in the screenshot — letters get smaller toward the bottom.
2. Read the highlighted/indicated line of text.
3. Type the letters exactly as shown using Fill.
4. Pay attention to case and spacing.

**Common confusions:** O vs 0, I vs l vs 1, S vs 5, B vs 8, Z vs 2.
Read carefully and type in the input field. Solve in 1-2 rounds.
"##;

const CREATIVE_SKILL: &str = r##"
Creative drawing: Draw or create something using mouse strokes.

**Strategy:**
1. Use ClickDragPoint to draw simple shapes (smiley face, house, star, etc.).
2. Keep it simple — a few recognizable strokes.
3. For a smiley: circle face, two dot eyes, curved mouth.

**Simple smiley face (common safe choice):**
Draw a circle for the head, two dots for eyes, a curve for mouth.
Use ClickDragPoint for each stroke. Solve in 2-3 rounds, then verify.
"##;

const NETWORK_SKILL: &str = r##"
Networking challenge: Connect nodes, people, or items together.

**Strategy:**
1. Look at the nodes/people displayed in the screenshot.
2. Identify which ones should be connected (check labels, colors, instructions).
3. Drag from one node to another using ClickDragPoint to create connections.
4. Or click pairs of nodes sequentially to link them.

Read the connection requirements from the instructions. Solve in 2-4 rounds.
"##;

const TRADING_SKILL: &str = r##"
Trading/timing challenge: Buy low, sell high at the right moment.

**Strategy:**
1. Watch the chart/price indicator in the screenshot.
2. Click BUY when the price is at a low point.
3. Click SELL when the price peaks.
4. Look for buttons labeled "Buy" or "Sell" or arrows.

**Timing approach:**
- If there's a moving line graph, click buy at valleys and sell at peaks.
- Use Wait between actions to let the price change.
- Target profit, not perfection.

Solve in 3-5 rounds with Wait between buy/sell actions.
"##;

const TEXT_CHOICE_SKILL: &str = r##"
Text choice challenge: Select or type a meaningful text response.

**Strategy:**
1. Read all available options in the screenshot.
2. Choose the most genuine/human/meaningful response.
3. If text input: type something thoughtful and human (not robotic).
4. Click the chosen option or submit text.

For philosophical questions: choose the empathetic, creative, or personal answer.
Solve in 1-2 rounds.
"##;

/// JS that reads sliding puzzle tile positions and computes moves.
const SLIDING_PUZZLE_PRE_EVALUATE_JS: &str = r##"try{
var tiles=[...document.querySelectorAll('[class*=tile],[class*=slide],[class*=puzzle] > div')].filter(function(t){var r=t.getBoundingClientRect();return r.width>20&&r.height>20;});
if(tiles.length>0){var info=tiles.map(function(t,i){var r=t.getBoundingClientRect();var txt=(t.textContent||'').trim().substring(0,5);return{id:i,x:Math.round(r.x),y:Math.round(r.y),w:Math.round(r.width),h:Math.round(r.height),txt:txt,cls:t.className.substring(0,30)};});
document.title='SLIDE:'+JSON.stringify({n:tiles.length,tiles:info});}
else{document.title='SLIDE_ERR:no_tiles';}
}catch(e){document.title='SLIDE_ERR:'+e.message;}
"##;

const SLIDING_PUZZLE_SKILL: &str = r##"
Sliding tile puzzle: Move tiles to solve the puzzle by sliding into the empty space.

Title format: `SLIDE:{"n":N,"tiles":[{"id":N,"x":N,"y":N,"txt":"1"}...]}`

**Strategy:**
1. Read tile positions from the title data.
2. Identify the empty space (missing tile in the grid).
3. Click a tile adjacent to the empty space to slide it in.
4. Work top-to-bottom, left-to-right: solve first row, then second row, etc.

**Click the tile you want to move** (it slides into the empty space):
```json
"steps": [
  {"ClickPoint":{"x":TILEx,"y":TILEy}},
  {"Wait":300},
  {"ClickPoint":{"x":TILE2x,"y":TILE2y}},
  {"Wait":300}
]
```

Make 2-4 moves per round. Verify when solved. Solve in 5-10 rounds.
"##;

const TRAFFIC_SIGNAL_SKILL: &str = r##"
Traffic signal challenge: Interact with traffic lights or signals.

**Strategy:**
1. Look at the traffic signals in the screenshot.
2. Click the green light, or set signals to the correct state.
3. Follow the instructions — may need to click specific colored lights.
4. If it's a tree of signals, click them in the correct order.

Click the indicated signal elements. Solve in 2-3 rounds.
"##;

const RHYTHM_SKILL: &str = r##"
Rhythm/drum challenge: Reproduce a rhythm or sound pattern.

**Strategy:**
1. First round: WATCH/LISTEN to the pattern being played. Note the order of drum hits.
2. Second round: Click the drums/elements in the SAME order and timing.
3. Each drum/pad is a clickable element — click them in sequence.

**Pattern reproduction:**
```json
"steps": [
  {"ClickPoint":{"x":DRUM1x,"y":DRUM1y}},
  {"Wait":300},
  {"ClickPoint":{"x":DRUM2x,"y":DRUM2y}},
  {"Wait":300},
  {"ClickPoint":{"x":DRUM1x,"y":DRUM1y}},
  {"Wait":300}
]
```

Watch the demo first (1 round), then reproduce (1-2 rounds). Solve in 3-4 rounds.
"##;

const BRAND_LOGO_SKILL: &str = r##"
Brand logo challenge: Identify brands from their logos.

**Strategy:**
1. Look at the logo image in the screenshot.
2. Identify the brand name (common: Apple, Nike, McDonald's, Google, Amazon, etc.).
3. Type the brand name or click the matching option.
4. Logos may be simplified, partial, or stylized.

**Common logo features:**
- Apple: bitten apple silhouette
- Nike: swoosh
- McDonald's: golden arches (M)
- Starbucks: green mermaid/siren
- Amazon: arrow from A to Z

Type the brand name in the input field. Solve in 1-2 rounds.
"##;

/// JS that reads a math expression from the page and computes the answer.
const MATH_PRE_EVALUATE_JS: &str = r##"try{
var expr='';var els=[...document.querySelectorAll('h1,h2,h3,.math,.equation,.captcha-text,.challenge-text,p,span')];
for(var el of els){var t=(el.textContent||'').trim();if(/[\d\+\-\*\/\=\^]+/.test(t)&&t.length>2&&t.length<100){expr=t;break;}}
if(expr){try{var clean=expr.replace(/[^0-9\+\-\*\/\.\(\)\^%]/g,' ').trim().replace(/\^/g,'**');var ans=Function('"use strict";return ('+clean+')')();document.title='MATH:'+JSON.stringify({expr:expr,answer:String(ans)});}catch(ee){document.title='MATH:'+JSON.stringify({expr:expr,answer:''});}}
else{document.title='MATH_ERR:no_expression';}
}catch(e){document.title='MATH_ERR:'+e.message;}
"##;

const MATH_SKILL: &str = r##"
Mathematics challenge: Solve a math equation or expression.

Title format: `MATH:{"expr":"2 + 3 * 4","answer":"14"}`

**Strategy:**
1. Read the math expression from the title (pre-evaluate extracts and solves it).
2. If answer is provided, type it directly using Fill.
3. If answer is empty, solve the expression yourself and type the result.
4. Submit via verify button.

```json
"steps": [
  {"Fill":{"selector":"input","value":"ANSWER"}},
  {"Wait":300},
  {"Click":"#captcha-verify-button"}
]
```

Solve in 1 round.
"##;

const SHUFFLE_SKILL: &str = r##"
Shuffle tracking: Track an object (card, cup, ball) as it's shuffled.

**Strategy:**
1. First: identify which item hides the target (ball under cup, marked card, etc.).
2. Watch the shuffle animation carefully in the screenshot sequence.
3. Track the position through each swap.
4. Click the final position of the target item.

**Tips:**
- Focus on one item only, ignore distractors.
- The shuffle usually involves 3-7 swaps.
- After shuffle ends, click the item at the tracked position.

Solve in 2-3 rounds (watch, then click).
"##;

const MATCH3_SKILL: &str = r##"
Match-3 puzzle: Swap adjacent tiles to create rows/columns of 3+ matching items.

**Strategy:**
1. Scan the grid for potential 3-in-a-row matches.
2. Swap two adjacent tiles by clicking the first, then the second.
3. Or drag one tile onto its neighbor.
4. Look for where ONE swap creates a match of 3+ identical items.

**Swap approach:**
```json
"steps": [
  {"ClickPoint":{"x":TILE1x,"y":TILE1y}},
  {"Wait":200},
  {"ClickPoint":{"x":TILE2x,"y":TILE2y}},
  {"Wait":500}
]
```

Make 1-2 swaps per round. Verify when required matches are complete. Solve in 3-5 rounds.
"##;

const ODD_ONE_OUT_SKILL: &str = r##"
Find the imposter: Identify the item that doesn't belong among similar items.

**Strategy:**
1. Look at ALL items in the grid carefully.
2. Most items look alike — find the ONE that's different.
3. Differences can be subtle: wrong color, mirrored, different expression, extra/missing detail.
4. Click the odd one out.

**Tips:**
- Compare items systematically: top-left vs top-right, etc.
- Look for: color differences, orientation, missing features, size.
- The difference is usually small but visible.

Click the imposter, then verify. Solve in 1-2 rounds.
"##;

const DECISION_SKILL: &str = r##"
Decision challenge: Choose between presented options.

**Strategy:**
1. Read all available options carefully.
2. Make a definitive choice — don't hesitate.
3. Click the chosen option.
4. For ethical dilemmas: choose the most commonly accepted answer.

Just pick an option and click it. No overthinking needed. Solve in 1 round.
"##;

const FACE_MATCHING_SKILL: &str = r##"
Face matching: Compare, match, or identify faces.

**Strategy:**
1. Look at the reference face(s) and the options.
2. Compare facial features: eyes, nose, mouth shape, hair, jawline.
3. Click the matching face or the correct answer.
4. Pay attention to subtle differences: eyebrow shape, ear size, chin.

Focus on distinctive features (glasses, facial hair, dimples). Solve in 1-2 rounds.
"##;

const SLOT_MACHINE_SKILL: &str = r##"
Slot machine: Stop the reels to match symbols.

**Strategy:**
1. Watch the spinning reels.
2. Click STOP or the reel itself to stop each one.
3. Try to align matching symbols across reels.
4. Timing is key — click when you see the target symbol.

**Approach:**
```json
"steps": [
  {"Click":"[class*=reel]:nth-child(1)"},
  {"Wait":500},
  {"Click":"[class*=reel]:nth-child(2)"},
  {"Wait":500},
  {"Click":"[class*=reel]:nth-child(3)"}
]
```

Or look for a "Spin" then "Stop" button. Solve in 2-4 rounds.
"##;

const DIG_FIND_SKILL: &str = r##"
Dig/find challenge: Search for something hidden by clicking or digging.

**Strategy:**
1. Click around the scene to dig/reveal hidden items.
2. Look for subtle visual clues (disturbed ground, different texture).
3. Click systematically across the area.
4. When you find the item, click verify.

Click multiple spots per round to search efficiently. Solve in 2-4 rounds.
"##;

const TURING_TEXT_SKILL: &str = r##"
Reverse Turing test: Type text that proves you're human, not a robot.

**Strategy:**
1. Find the text input field.
2. Type something genuinely human — personal, creative, emotional.
3. Good examples: "I love the smell of rain on warm pavement" or "My grandma's cookies always made me smile"
4. Avoid robotic/formal language. Be casual and personal.

```json
"steps": [
  {"Fill":{"selector":"input,textarea","value":"I love watching sunsets while eating ice cream — the orange sky reminds me of childhood summers."}},
  {"Wait":300},
  {"Click":"#captcha-verify-button"}
]
```

Be authentic, not formulaic. Solve in 1-2 rounds.
"##;

const ASSEMBLY_SKILL: &str = r##"
Assembly/identification challenge: Identify items from instructions or blueprints.

**Strategy:**
1. Look at the assembly instructions or parts diagram in the screenshot.
2. Identify the furniture/item being assembled.
3. Select the correct item name or click the matching option.
4. For drag challenges: drag parts to their correct positions.

Match shapes and silhouettes to identify items. Solve in 1-3 rounds.
"##;

const CHESS_SKILL: &str = r##"
Chess challenge: Make the best move or solve a chess puzzle.

**Strategy:**
1. Read the board position from the screenshot.
2. Identify: whose turn it is, which pieces are where.
3. Look for: checkmate in 1, forks, pins, skewers, hanging pieces.
4. Click the piece to move, then click the destination square.

**Common tactics:**
- Queen + Rook battery for back-rank mate
- Knight forks (attacking 2+ pieces)
- Bishop pins against the king
- Pawn promotion threats

**Steps:**
```json
"steps": [
  {"ClickPoint":{"x":PIECEx,"y":PIECEy}},
  {"Wait":300},
  {"ClickPoint":{"x":DESTx,"y":DESTy}},
  {"Wait":500}
]
```

Think carefully before moving. Solve in 3-8 rounds.
"##;

const FIND_PERSON_SKILL: &str = r##"
Find person: Locate a specific named person in a crowd or scene.

**Strategy:**
1. Read the person's name from the title/instructions.
2. Scan the screenshot for people — look at name tags, labels, distinctive features.
3. Click on the correct person.
4. If there are name labels, match the text exactly.

Look for text labels near each person. Click the correct one. Solve in 1-2 rounds.
"##;

const FLOOR_NAV_SKILL: &str = r##"
Floor navigation: Navigate through building floors to reach a target.

**Strategy:**
1. Read which floor you need to reach.
2. Click elevator buttons, stairs, or floor selectors.
3. Navigate up or down to the target floor.
4. Click the door/room at the target floor.

Look for UP/DOWN arrows or floor number buttons. Solve in 2-4 rounds.
"##;

const BELL_PATTERN_SKILL: &str = r##"
Bell/sound pattern: Reproduce a sequence of bell rings or sounds.

**Strategy:**
1. First round: observe the pattern being demonstrated (which bells ring in which order).
2. Note the sequence: left-right-left, or numbered positions.
3. Reproduce by clicking bells/elements in the same order.

**Pattern reproduction:**
```json
"steps": [
  {"ClickPoint":{"x":BELL1x,"y":BELL1y}},
  {"Wait":400},
  {"ClickPoint":{"x":BELL2x,"y":BELL2y}},
  {"Wait":400},
  {"ClickPoint":{"x":BELL3x,"y":BELL3y}}
]
```

Match the exact sequence and timing. Solve in 2-4 rounds.
"##;

const FINAL_CREATIVE_SKILL: &str = r##"
Final challenge: Complete the last creative task.

**Strategy:**
1. Read the instructions carefully from the screenshot.
2. This is usually a unique creative challenge.
3. Follow the specific instructions — it may involve typing, drawing, or clicking.
4. Be creative and genuine in your response.

Adapt to whatever the challenge asks. Look at all interactive elements.
Solve in 2-4 rounds.
"##;

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
        let registry = builtin_web_challenges();
        assert!(registry.len() >= 14);

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
        let skill = Skill::new("test", "test skill").with_pre_evaluate("document.title='HELLO'");
        assert_eq!(
            skill.pre_evaluate.as_deref(),
            Some("document.title='HELLO'")
        );

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
        let registry = builtin_web_challenges();
        let ttt = registry
            .get("tic-tac-toe")
            .expect("tic-tac-toe skill missing");
        assert!(
            ttt.pre_evaluate.is_some(),
            "TTT should have pre_evaluate JS"
        );
        let js = ttt.pre_evaluate.as_deref().unwrap();
        assert!(
            js.contains("TTT:"),
            "TTT pre_evaluate should set title with TTT: prefix"
        );
        assert!(
            js.contains("cell-selected"),
            "TTT pre_evaluate should use cell-selected selector"
        );
        assert!(
            js.contains("cell-disabled"),
            "TTT pre_evaluate should use cell-disabled selector"
        );
        assert!(
            js.contains("TTT_ERR"),
            "TTT pre_evaluate should have error handling"
        );
    }

    #[test]
    fn test_builtin_word_search_has_pre_evaluate() {
        let registry = builtin_web_challenges();
        let ws = registry
            .get("word-search")
            .expect("word-search skill missing");
        assert!(
            ws.pre_evaluate.is_some(),
            "Word search should have pre_evaluate JS"
        );
        let js = ws.pre_evaluate.as_deref().unwrap();
        assert!(
            js.contains("WS:"),
            "WS pre_evaluate should set title with WS: prefix"
        );
        assert!(
            js.contains("word-search-grid-item"),
            "WS pre_evaluate should use word-search-grid-item selector"
        );
        assert!(
            js.contains("WS_ERR"),
            "WS pre_evaluate should have error handling"
        );
    }

    #[test]
    fn test_builtin_ttt_triggers_fixed() {
        let registry = builtin_web_challenges();
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
        let registry = builtin_web_challenges();

        // TTT simplified content should NOT contain Evaluate JS
        let ttt = registry.get("tic-tac-toe").unwrap();
        assert!(ttt.content.contains("Do NOT write any Evaluate JS"));
        assert!(!ttt.content.contains("querySelectorAll"));

        // Word search is self-solving via pre_evaluate
        let ws = registry.get("word-search").unwrap();
        assert!(
            ws.content.contains("auto-dragged"),
            "WS skill should mention auto-drag"
        );
        assert!(ws.pre_evaluate.is_some(), "WS should have pre_evaluate JS");

        // Rotation simplified content should NOT contain Evaluate JS
        let rot = registry.get("rotation-puzzle").unwrap();
        assert!(rot.content.contains("Do NOT write any Evaluate JS"));
        assert!(!rot.content.contains("querySelectorAll"));
        assert!(
            rot.pre_evaluate.is_some(),
            "Rotation should have pre_evaluate JS"
        );

        assert!(
            ws.content.contains("Do NOT write any drag"),
            "WS skill should prohibit manual drag JS"
        );
    }

    #[test]
    fn test_skills_without_pre_evaluate_unchanged() {
        let registry = builtin_web_challenges();

        // Image grid, text-captcha, slider, checkbox should NOT have pre_evaluate
        for name in &[
            "image-grid-selection",
            "text-captcha",
            "slider-drag",
            "checkbox-click",
            "license-plate",
            "nested-grid",
            "find-waldo",
            "chihuahua-muffin",
            "reverse-selection",
            "affirmations",
        ] {
            let skill = registry
                .get(name)
                .unwrap_or_else(|| panic!("{} missing", name));
            assert!(
                skill.pre_evaluate.is_none(),
                "{} should NOT have pre_evaluate",
                name
            );
        }
    }

    #[test]
    fn test_new_level_skills_present() {
        let registry = builtin_web_challenges();

        // L8: License plate
        let lp = registry.get("license-plate").expect("license-plate missing");
        assert!(lp.matches("", "License Plate Challenge", ""));
        assert!(lp.content.contains("license plate"));

        // L9: Nested grid
        let nested = registry.get("nested-grid").expect("nested-grid missing");
        assert!(nested.matches("", "Nested Squares", ""));
        assert!(nested.content.contains("SUBDIVIDE"));
        assert!(nested.priority > 5, "nested-grid should override image-grid");

        // L10: Whack-a-mole
        let wam = registry.get("whack-a-mole").expect("whack-a-mole missing");
        assert!(wam.matches("", "Whack a Mole!", ""));
        assert!(wam.pre_evaluate.is_some(), "WAM should have pre_evaluate");
        let js = wam.pre_evaluate.as_deref().unwrap();
        assert!(js.contains("WAM:"), "WAM pre_evaluate should set WAM: prefix");

        // L11: Waldo
        let waldo = registry.get("find-waldo").expect("find-waldo missing");
        assert!(waldo.matches("", "Where's Waldo", ""));
        assert!(waldo.content.contains("striped"));

        // L12: Chihuahua vs muffin
        let cm = registry.get("chihuahua-muffin").expect("chihuahua-muffin missing");
        assert!(cm.matches("", "Muffins? Or Chihuahuas", ""));
        assert!(cm.content.contains("CHIHUAHUAS"));

        // L13: Reverse selection
        let rev = registry.get("reverse-selection").expect("reverse-selection missing");
        assert!(rev.matches("", "Reverse CAPTCHA", ""));
        assert!(rev.content.contains("OPPOSITE"));

        // L14: Affirmations
        let aff = registry.get("affirmations").expect("affirmations missing");
        assert!(aff.matches("", "Affirmations Level", ""));
        assert!(aff.content.contains("I'm not a robot"));
    }

    #[test]
    fn test_nested_overrides_image_grid() {
        let registry = builtin_web_challenges();
        let nested = registry.get("nested-grid").unwrap();
        let img_grid = registry.get("image-grid-selection").unwrap();
        assert!(
            nested.priority > img_grid.priority,
            "nested-grid priority ({}) should be higher than image-grid ({})",
            nested.priority,
            img_grid.priority
        );
    }
}
