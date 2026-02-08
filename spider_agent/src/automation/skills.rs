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
/// Loads all skills from `spider_skills::web_challenges::registry()` and then
/// adds engine-specific `pre_evaluate` JavaScript for skills that need it.
/// Pre-evaluate JS runs in the browser before the LLM sees the page,
/// extracting puzzle state automatically.
pub fn builtin_web_challenges() -> SkillRegistry {
    // Load all skills from the spider_skills crate (110 skills)
    let mut registry = spider_skills::web_challenges::registry();

    // Add pre_evaluate JS overlays for skills that need engine-side computation.
    // These run before the LLM inference round to extract puzzle state into document.title.

    if let Some(skill) = registry.get_mut("rotation-puzzle") {
        skill.pre_evaluate = Some(ROTATION_PRE_EVALUATE_JS.to_string());
    }

    if let Some(skill) = registry.get_mut("tic-tac-toe") {
        skill.pre_evaluate = Some(TTT_PRE_EVALUATE_JS.to_string());
    }

    if let Some(skill) = registry.get_mut("word-search") {
        skill.pre_evaluate = Some(WORD_SEARCH_PRE_EVALUATE_JS.to_string());
    }

    if let Some(skill) = registry.get_mut("nested-grid") {
        skill.pre_evaluate = Some(NESTED_PRE_EVALUATE_JS.to_string());
    }

    if let Some(skill) = registry.get_mut("whack-a-mole") {
        skill.pre_evaluate = Some(WHACK_A_MOLE_PRE_EVALUATE_JS.to_string());
    }

    if let Some(skill) = registry.get_mut("draw-circle") {
        skill.pre_evaluate = Some(DRAW_CIRCLE_PRE_EVALUATE_JS.to_string());
    }

    if let Some(skill) = registry.get_mut("sliding-puzzle") {
        skill.pre_evaluate = Some(SLIDING_PUZZLE_PRE_EVALUATE_JS.to_string());
    }

    if let Some(skill) = registry.get_mut("math-solver") {
        skill.pre_evaluate = Some(MATH_PRE_EVALUATE_JS.to_string());
    }

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

// ─── Pre-evaluate JS constants ──────────────────────────────────────────
//
// These JS snippets run in the browser BEFORE the LLM sees the page.
// They extract puzzle state into document.title so the model can read it.
// Skill content lives in spider_skills crate (.md files via include_str!).

/// JS executed by the engine before the LLM sees the rotation page.
/// Reads each tile's CSS transform, computes clicks needed, writes to title.
/// Does NOT click — the model uses real Click actions based on the title info.
const ROTATION_PRE_EVALUATE_JS: &str = "try{const t=[...document.querySelectorAll('.rotating-item')];const n=t.length;const tiles=t.map((e,i)=>{const m=getComputedStyle(e).transform;let c=0;if(m&&m!=='none'){const v=m.match(/matrix\\(([^)]+)\\)/);if(v){const p=v[1].split(',').map(Number);const a=Math.round(Math.atan2(p[1],p[0])*180/Math.PI);c=a>45&&a<135?3:Math.abs(a)>135?2:a<-45&&a>-135?1:0;}}return{i,c};});const done=tiles.every(t=>t.c===0);const clicks=tiles.filter(t=>t.c>0).map(t=>t.i+':'+t.c).join(',');document.title='ROT:'+JSON.stringify({n,done,clicks});}catch(e){document.title='ROT_ERR:'+e.message;}";

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

/// JS that reads sliding puzzle tile positions and computes moves.
const SLIDING_PUZZLE_PRE_EVALUATE_JS: &str = r##"try{
var tiles=[...document.querySelectorAll('[class*=tile],[class*=slide],[class*=puzzle] > div')].filter(function(t){var r=t.getBoundingClientRect();return r.width>20&&r.height>20;});
if(tiles.length>0){var info=tiles.map(function(t,i){var r=t.getBoundingClientRect();var txt=(t.textContent||'').trim().substring(0,5);return{id:i,x:Math.round(r.x),y:Math.round(r.y),w:Math.round(r.width),h:Math.round(r.height),txt:txt,cls:t.className.substring(0,30)};});
document.title='SLIDE:'+JSON.stringify({n:tiles.length,tiles:info});}
else{document.title='SLIDE_ERR:no_tiles';}
}catch(e){document.title='SLIDE_ERR:'+e.message;}
"##;

/// JS that reads a math expression from the page and computes the answer.
const MATH_PRE_EVALUATE_JS: &str = r##"try{
var expr='';var els=[...document.querySelectorAll('h1,h2,h3,.math,.equation,.captcha-text,.challenge-text,p,span')];
for(var el of els){var t=(el.textContent||'').trim();if(/[\d\+\-\*\/\=\^]+/.test(t)&&t.length>2&&t.length<100){expr=t;break;}}
if(expr){try{var clean=expr.replace(/[^0-9\+\-\*\/\.\(\)\^%]/g,' ').trim().replace(/\^/g,'**');var ans=Function('"use strict";return ('+clean+')')();document.title='MATH:'+JSON.stringify({expr:expr,answer:String(ans)});}catch(ee){document.title='MATH:'+JSON.stringify({expr:expr,answer:''});}}
else{document.title='MATH_ERR:no_expression';}
}catch(e){document.title='MATH_ERR:'+e.message;}
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
        assert!(registry.len() >= 110);

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

        // These skills should NOT have pre_evaluate (content-only skills)
        for name in &[
            "image-grid-selection",
            "text-captcha",
            "slider-drag",
            "checkbox-click",
            "license-plate",
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
