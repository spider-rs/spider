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
        let registry = builtin_web_challenges();
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

        // Word search may allow Evaluate as fallback when words aren't auto-detected
        let ws = registry.get("word-search").unwrap();
        assert!(
            ws.content.contains("ClickDragPoint"),
            "WS skill should mention ClickDragPoint"
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
}
