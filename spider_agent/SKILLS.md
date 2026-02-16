# Skills Integration (spider_agent)

How `spider_agent` consumes and extends skills from the `spider_skills` crate.

## Architecture

```
spider_skills (v0.1.7)              spider_agent
┌─────────────────────┐            ┌──────────────────────────────┐
│ 110 skills          │            │ automation/skills.rs         │
│ (.md via include_str)│──import──▶│  builtin_web_challenges()    │
│ triggers + content  │            │  + pre_evaluate JS overlays  │
│ web_challenges.rs   │            │                              │
└─────────────────────┘            │ automation/browser.rs        │
                                   │  per-round skill injection   │
                                   │  pre_evaluate execution      │
                                   │  engine game loops           │
                                   └──────────────────────────────┘
```

`spider_agent` does NOT define any skill content — all prompt text lives in `spider_skills`. The agent only adds:
1. **Pre-evaluate JS** — Browser-side computation that runs before LLM inference
2. **Engine game loops** — Tight CDP loops for time-sensitive challenges (TTT, whack-a-mole, word search drag, nested grid click, draw circle)
3. **Skill injection** — Per-round matching and context injection into the LLM prompt

## Feature Flags

| Flag | Crate | Enables |
|------|-------|---------|
| `skills` | spider_agent | `spider_skills/web_challenges` dependency |
| `skills_s3` | spider_agent | S3 skill loading (`skills` + `aws-sdk-s3`) |
| `agent_skills` | spider | `agent` + `spider_agent/skills` |
| `agent_skills_s3` | spider | `agent_skills` + `spider_agent/skills_s3` |
| `agent_full` | spider | Everything including skills |

```toml
# In spider/Cargo.toml — enable skills
spider = { version = "2", features = ["agent_skills"] }

# Or with S3 loading
spider = { version = "2", features = ["agent_skills_s3"] }
```

## Pre-Evaluate JS Overlays

`builtin_web_challenges()` in `automation/skills.rs` loads all 110 skills from `spider_skills::web_challenges::registry()`, then patches 8 skills with engine-specific `pre_evaluate` JavaScript:

| Skill | JS Prefix | What It Does |
|-------|-----------|-------------|
| `rotation-puzzle` | `ROT:` | Reads CSS transform matrix per tile, computes clicks needed |
| `tic-tac-toe` | `TTT:` | Reads board state, computes optimal move (minimax), auto-clicks via dispatchEvent |
| `word-search` | `WS:` / `WS_DRAG:` | Extracts letter grid, finds all words algorithmically, outputs drag coordinates |
| `nested-grid` | `NEST:` | Detects stop sign image, computes bounding-rect overlap with leaf boxes |
| `whack-a-mole` | `WAM:` | Detects visible moles, auto-clicks them via dispatchEvent |
| `draw-circle` | `CIRCLE:` | Finds canvas/draw area, computes circular path coordinates |
| `sliding-puzzle` | `SLIDE:` | Reads tile positions and classes for puzzle solver |
| `math-solver` | `MATH:` | Extracts math expression from page, evaluates answer |

### How Pre-Evaluate Works

Each round, before the LLM sees the page:

1. `find_pre_evaluates(url, title, html)` returns matching skills with JS payloads
2. Engine runs each JS via `page.evaluate()` in the browser
3. JS writes structured results to `document.title` (e.g., `TTT:{"board":"M..T.....","best":4}`)
4. LLM reads the modified title and emits simple actions (Click, Wait) instead of writing extraction code

```
Browser Page ──▶ pre_evaluate JS ──▶ Modified Title ──▶ LLM reads title ──▶ Click actions
```

This eliminates the model's tendency to write incorrect DOM selectors or extraction code.

### Title Prefixes

| Prefix | Skill | Data Format |
|--------|-------|-------------|
| `ROT:{...}` | rotation-puzzle | `{n, done, clicks:"idx:count,..."}` |
| `TTT:{...}` | tic-tac-toe | `{n, board, best, clicked, myWin, thWin, full}` |
| `WS:{...}` | word-search | `{n, rows, cols, grid, words, found}` |
| `WS_DRAG:{...}` | word-search | `{n, words, drags}` (engine auto-drags) |
| `NEST:{...}` | nested-grid | `{total, sel, toClick, selected, hasSign, boxes}` |
| `WAM:{...}` | whack-a-mole | `{moles, clicked}` |
| `CIRCLE:{...}` | draw-circle | `{cx, cy, rad, pts}` |
| `SLIDE:{...}` | sliding-puzzle | `{n, tiles:[{id,x,y,w,h,txt,cls}]}` |
| `MATH:{...}` | math-solver | `{expr, answer}` |
| `*_ERR:msg` | any | Error during pre_evaluate |

## Engine Game Loops

For time-sensitive challenges, the engine runs tight loops via CDP without waiting for LLM rounds:

### Tic-Tac-Toe Loop
- Plays entire game in ~0.7s/move tight loop
- Up to 3 games (handles draws by waiting for auto-reset)
- Clicks verify button on win
- Uses `dispatchEvent` for clicks (works on this specific page)

### Word Search CDP Drag
- Engine dispatches `MousePressed` → `MouseMoved` → `MouseReleased` through cell centers
- `buttons: 1` bitmask required during drag moves
- DOM marker `#ws-engine-done` prevents re-drag

### Nested Grid Auto-Click
- Computes bounding-rect overlap between stop sign image and leaf boxes
- Auto-clicks matching boxes via `ClickPoint`
- Handles recursive subdivision (clicking splits boxes into smaller grids)

### Whack-a-Mole Loop
- Detects visible moles via class/src/style checks
- Auto-clicks via `dispatchEvent` (pointerdown → mousedown → pointerup → mouseup → click)
- Runs every pre_evaluate round

### Draw Circle Path
- Computes 36-point circular path around canvas center
- Engine dispatches CDP mouse events along the path

## Per-Round Skill Injection

In `automation/browser.rs` (~line 709), each inference round:

1. **Match skills** — `registry.match_context_limited(url, title, html, 3, 4000)` returns combined prompt for up to 3 matching skills within 4000 chars
2. **Run pre_evaluate** — For matched skills with `pre_evaluate`, execute JS in browser
3. **Inject context** — Matched skill content is appended to `system_prompt_extra` for that round
4. **Stagnation check** — Raw title (before pre_evaluate) used for stuck detection so pre_evaluate doesn't mask stagnation

### Context Budget

| Parameter | Default | Description |
|-----------|---------|-------------|
| `max_skills` | 3 | Maximum skills injected per round |
| `max_chars` | 4000 | Maximum total chars for skill context |

Skills are sorted by priority (descending). If adding the next skill would exceed `max_chars`, it's skipped.

## Agent-Driven Skill Loading

When no skills auto-match, the agent can request skills via the `request_skill` memory operation. A skill catalog is injected into the prompt listing available skill names. The model can then request a specific skill by name.

## S3 Skill Loading

Load custom skills from S3-compatible storage (AWS S3, MinIO, Cloudflare R2):

```rust
use spider_agent::automation::skills::{S3SkillSource, load_from_s3, builtin_web_challenges};

let mut registry = builtin_web_challenges();
let source = S3SkillSource::new("my-bucket", "skills/")
    .with_region("us-east-1")
    .with_endpoint_url("https://r2.example.com"); // optional, for R2/MinIO

let loaded = load_from_s3(&mut registry, &source).await?;
// S3 skills override built-in skills with the same name
```

Or combine in one call:

```rust
use spider_agent::automation::skills::{S3SkillSource, with_builtin_and_s3};

let source = S3SkillSource::new("my-bucket", "skills/");
let registry = with_builtin_and_s3(&source).await?;
```

## Adding Custom Pre-Evaluate JS

To add engine-side computation for a custom or existing skill:

```rust
let mut registry = builtin_web_challenges();

if let Some(skill) = registry.get_mut("my-skill") {
    skill.pre_evaluate = Some(r#"
        try {
            // Extract state from the page
            const data = /* ... */;
            document.title = 'MYPREFIX:' + JSON.stringify(data);
        } catch(e) {
            document.title = 'MYPREFIX_ERR:' + e.message;
        }
    "#.to_string());
}
```

Guidelines for pre_evaluate JS:
- Always wrap in `try/catch`, write errors to title with `_ERR:` suffix
- Write results to `document.title` with a unique prefix
- Keep JS compact — it runs every matching round
- Use `dispatchEvent` only when the target page handles custom events (rare). Prefer having the LLM emit real Click/ClickPoint actions.
- Check for a "done" marker element to short-circuit repeated runs: `if(document.getElementById('my-done')){...}`
