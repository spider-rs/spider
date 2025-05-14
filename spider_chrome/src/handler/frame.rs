use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::map::Entry;

use chromiumoxide_cdp::cdp::browser_protocol::network::LoaderId;
use chromiumoxide_cdp::cdp::browser_protocol::page::{
    AddScriptToEvaluateOnNewDocumentParams, CreateIsolatedWorldParams, EventFrameDetached,
    EventFrameStartedLoading, EventFrameStoppedLoading, EventLifecycleEvent,
    EventNavigatedWithinDocument, Frame as CdpFrame, FrameTree,
};
use chromiumoxide_cdp::cdp::browser_protocol::target::EventAttachedToTarget;
use chromiumoxide_cdp::cdp::js_protocol::runtime::*;
use chromiumoxide_cdp::cdp::{
    browser_protocol::page::{self, FrameId},
    // js_protocol::runtime,
};
use chromiumoxide_types::{Method, MethodId, Request};

use crate::error::DeadlineExceeded;
use crate::handler::domworld::DOMWorld;
use crate::handler::http::HttpRequest;
use crate::handler::REQUEST_TIMEOUT;
use crate::{cmd::CommandChain, ArcHttpRequest};

const EVALUATION_SCRIPT_URL: &str = "____chromiumoxide_utility_world___evaluation_script__";

// lazy_static::lazy_static! {
//     /// Spoof the runtime.
//     static ref CHROME_SPOOF_RUNTIME: bool = {
//         std::env::var("CHROME_SPOOF_RUNTIME").unwrap_or_else(|_| "false".to_string()) == "true"
//     };
// }

/// Generate a collision-resistant world name using `id` + randomness.
pub fn random_world_name(id: &str) -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let rand_len = rng.random_range(6..=12);

    // Convert first few chars of id into base36-compatible chars
    let id_part: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(5)
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_alphabetic() {
                c
            } else {
                // convert 0-9 into a base36 letter offset to obscure it a bit
                (b'a' + (c as u8 - b'0') % 26) as char
            }
        })
        .collect();

    // Generate random base36 tail
    let rand_part: String = (0..rand_len)
        .filter_map(|_| std::char::from_digit(rng.random_range(0..36), 36))
        .collect();

    // Ensure first char is always a letter (10–35 => a–z)
    let first = std::char::from_digit(rng.random_range(10..36), 36).unwrap_or('a');

    format!("{first}{id_part}{rand_part}")
}

/// Represents a frame on the page
#[derive(Debug)]
pub struct Frame {
    parent_frame: Option<FrameId>,
    /// Cdp identifier of this frame
    id: FrameId,
    main_world: DOMWorld,
    secondary_world: DOMWorld,
    loader_id: Option<LoaderId>,
    /// Current url of this frame
    url: Option<String>,
    /// The http request that loaded this with this frame
    http_request: ArcHttpRequest,
    /// The frames contained in this frame
    child_frames: HashSet<FrameId>,
    name: Option<String>,
    /// The received lifecycle events
    lifecycle_events: HashSet<MethodId>,
    isolated_world_name: String,
}

impl Frame {
    pub fn new(id: FrameId) -> Self {
        let isolated_world_name = random_world_name(id.inner());

        Self {
            parent_frame: None,
            id,
            main_world: Default::default(),
            secondary_world: Default::default(),
            loader_id: None,
            url: None,
            http_request: None,
            child_frames: Default::default(),
            name: None,
            lifecycle_events: Default::default(),
            isolated_world_name,
        }
    }

    pub fn with_parent(id: FrameId, parent: &mut Frame) -> Self {
        parent.child_frames.insert(id.clone());
        Self {
            parent_frame: Some(parent.id.clone()),
            id,
            main_world: Default::default(),
            secondary_world: Default::default(),
            loader_id: None,
            url: None,
            http_request: None,
            child_frames: Default::default(),
            name: None,
            lifecycle_events: Default::default(),
            isolated_world_name: parent.isolated_world_name.clone(),
        }
    }

    pub fn get_isolated_world_name(&self) -> &String {
        &self.isolated_world_name
    }

    pub fn parent_id(&self) -> Option<&FrameId> {
        self.parent_frame.as_ref()
    }

    pub fn id(&self) -> &FrameId {
        &self.id
    }

    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn main_world(&self) -> &DOMWorld {
        &self.main_world
    }

    pub fn secondary_world(&self) -> &DOMWorld {
        &self.secondary_world
    }

    pub fn lifecycle_events(&self) -> &HashSet<MethodId> {
        &self.lifecycle_events
    }

    pub fn http_request(&self) -> Option<&Arc<HttpRequest>> {
        self.http_request.as_ref()
    }

    fn navigated(&mut self, frame: &CdpFrame) {
        self.name.clone_from(&frame.name);
        let url = if let Some(ref fragment) = frame.url_fragment {
            format!("{}{fragment}", frame.url)
        } else {
            frame.url.clone()
        };
        self.url = Some(url);
    }

    fn navigated_within_url(&mut self, url: String) {
        self.url = Some(url)
    }

    fn on_loading_stopped(&mut self) {
        self.lifecycle_events.insert("DOMContentLoaded".into());
        self.lifecycle_events.insert("load".into());
    }

    fn on_loading_started(&mut self) {
        self.lifecycle_events.clear();
        self.http_request.take();
    }

    pub fn is_loaded(&self) -> bool {
        self.lifecycle_events.contains("load")
    }

    pub fn clear_contexts(&mut self) {
        self.main_world.take_context();
        self.secondary_world.take_context();
    }

    pub fn destroy_context(&mut self, ctx_unique_id: &str) {
        if self.main_world.execution_context_unique_id() == Some(ctx_unique_id) {
            self.main_world.take_context();
        } else if self.secondary_world.execution_context_unique_id() == Some(ctx_unique_id) {
            self.secondary_world.take_context();
        }
    }

    pub fn execution_context(&self) -> Option<ExecutionContextId> {
        self.main_world.execution_context()
    }

    pub fn set_request(&mut self, request: HttpRequest) {
        self.http_request = Some(Arc::new(request))
    }
}

/// Maintains the state of the pages frame and listens to events produced by
/// chromium targeting the `Target`. Also listens for events that indicate that
/// a navigation was completed
#[derive(Debug)]
pub struct FrameManager {
    main_frame: Option<FrameId>,
    frames: HashMap<FrameId, Frame>,
    /// The contexts mapped with their frames
    context_ids: HashMap<String, FrameId>,
    isolated_worlds: HashSet<String>,
    /// Timeout after which an anticipated event (related to navigation) doesn't
    /// arrive results in an error
    request_timeout: Duration,
    /// Track currently in progress navigation
    pending_navigations: VecDeque<(FrameRequestedNavigation, NavigationWatcher)>,
    /// The currently ongoing navigation
    navigation: Option<(NavigationWatcher, Instant)>,
}

impl FrameManager {
    pub fn new(request_timeout: Duration) -> Self {
        FrameManager {
            main_frame: None,
            frames: Default::default(),
            context_ids: Default::default(),
            isolated_worlds: Default::default(),
            request_timeout,
            pending_navigations: Default::default(),
            navigation: None,
        }
    }

    /// The commands to execute in order to initialize this frame manager
    pub fn init_commands(timeout: Duration) -> CommandChain {
        let enable = page::EnableParams::default();
        let get_tree = page::GetFrameTreeParams::default();
        let set_lifecycle = page::SetLifecycleEventsEnabledParams::new(true);
        // let enable_runtime = EnableParams::default();
        // let disable_runtime = DisableParams::default();

        let mut commands = Vec::with_capacity(3);

        let enable_id = enable.identifier();
        let get_tree_id = get_tree.identifier();
        let set_lifecycle_id = set_lifecycle.identifier();
        // let enable_runtime_id = enable_runtime.identifier();
        // let disable_runtime_id = disable_runtime.identifier();

        if let Ok(value) = serde_json::to_value(enable) {
            commands.push((enable_id, value));
        }

        if let Ok(value) = serde_json::to_value(get_tree) {
            commands.push((get_tree_id, value));
        }

        if let Ok(value) = serde_json::to_value(set_lifecycle) {
            commands.push((set_lifecycle_id, value));
        }

        // if let Ok(value) = serde_json::to_value(enable_runtime) {
        //     commands.push((enable_runtime_id, value));
        // }

        // if let Ok(value) = serde_json::to_value(disable_runtime) {
        //     commands.push((disable_runtime_id, value));
        // }

        CommandChain::new(commands, timeout)
    }

    pub fn main_frame(&self) -> Option<&Frame> {
        self.main_frame.as_ref().and_then(|id| self.frames.get(id))
    }

    pub fn main_frame_mut(&mut self) -> Option<&mut Frame> {
        if let Some(id) = self.main_frame.as_ref() {
            self.frames.get_mut(id)
        } else {
            None
        }
    }

    /// Get the main isolated world name.
    pub fn get_isolated_world_name(&self) -> Option<&String> {
        self.main_frame
            .as_ref()
            .and_then(|id| match self.frames.get(id) {
                Some(fid) => Some(fid.get_isolated_world_name()),
                _ => None,
            })
    }

    pub fn frames(&self) -> impl Iterator<Item = &Frame> + '_ {
        self.frames.values()
    }

    pub fn frame(&self, id: &FrameId) -> Option<&Frame> {
        self.frames.get(id)
    }

    fn check_lifecycle(&self, watcher: &NavigationWatcher, frame: &Frame) -> bool {
        watcher.expected_lifecycle.iter().all(|ev| {
            frame.lifecycle_events.contains(ev)
                || (frame.url.is_none() && frame.lifecycle_events.contains("DOMContentLoaded"))
        }) && frame
            .child_frames
            .iter()
            .filter_map(|f| self.frames.get(f))
            .all(|f| self.check_lifecycle(watcher, f))
    }

    fn check_lifecycle_complete(
        &self,
        watcher: &NavigationWatcher,
        frame: &Frame,
    ) -> Option<NavigationOk> {
        if !self.check_lifecycle(watcher, frame) {
            return None;
        }
        if frame.loader_id == watcher.loader_id && !watcher.same_document_navigation {
            return None;
        }
        if watcher.same_document_navigation {
            return Some(NavigationOk::SameDocumentNavigation(watcher.id));
        }
        if frame.loader_id != watcher.loader_id {
            return Some(NavigationOk::NewDocumentNavigation(watcher.id));
        }
        None
    }

    /// Track the request in the frame
    pub fn on_http_request_finished(&mut self, request: HttpRequest) {
        if let Some(id) = request.frame.as_ref() {
            if let Some(frame) = self.frames.get_mut(id) {
                frame.set_request(request);
            }
        }
    }

    pub fn poll(&mut self, now: Instant) -> Option<FrameEvent> {
        // check if the navigation completed
        if let Some((watcher, deadline)) = self.navigation.take() {
            if now > deadline {
                // navigation request timed out
                return Some(FrameEvent::NavigationResult(Err(
                    NavigationError::Timeout {
                        err: DeadlineExceeded::new(now, deadline),
                        id: watcher.id,
                    },
                )));
            }

            if let Some(frame) = self.frames.get(&watcher.frame_id) {
                if let Some(nav) = self.check_lifecycle_complete(&watcher, frame) {
                    // request is complete if the frame's lifecycle is complete = frame received all
                    // required events
                    return Some(FrameEvent::NavigationResult(Ok(nav)));
                } else {
                    // not finished yet
                    self.navigation = Some((watcher, deadline));
                }
            } else {
                return Some(FrameEvent::NavigationResult(Err(
                    NavigationError::FrameNotFound {
                        frame: watcher.frame_id,
                        id: watcher.id,
                    },
                )));
            }
        } else if let Some((req, watcher)) = self.pending_navigations.pop_front() {
            // queue in the next navigation that is must be fulfilled until `deadline`
            let deadline = Instant::now() + req.timeout;
            self.navigation = Some((watcher, deadline));
            return Some(FrameEvent::NavigationRequest(req.id, req.req));
        }
        None
    }

    /// Entrypoint for page navigation
    pub fn goto(&mut self, req: FrameRequestedNavigation) {
        if let Some(frame_id) = &self.main_frame {
            self.navigate_frame(frame_id.clone(), req);
        }
    }

    /// Navigate a specific frame
    pub fn navigate_frame(&mut self, frame_id: FrameId, mut req: FrameRequestedNavigation) {
        let loader_id = self.frames.get(&frame_id).and_then(|f| f.loader_id.clone());
        let watcher = NavigationWatcher::until_page_load(req.id, frame_id.clone(), loader_id);

        // insert the frame_id in the request if not present
        req.set_frame_id(frame_id);

        self.pending_navigations.push_back((req, watcher))
    }

    /// Fired when a frame moved to another session
    pub fn on_attached_to_target(&mut self, _event: &EventAttachedToTarget) {
        // _onFrameMoved
    }

    pub fn on_frame_tree(&mut self, frame_tree: FrameTree) {
        self.on_frame_attached(
            frame_tree.frame.id.clone(),
            frame_tree.frame.parent_id.clone().map(Into::into),
        );
        self.on_frame_navigated(&frame_tree.frame);
        if let Some(children) = frame_tree.child_frames {
            for child_tree in children {
                self.on_frame_tree(child_tree);
            }
        }
    }

    pub fn on_frame_attached(&mut self, frame_id: FrameId, parent_frame_id: Option<FrameId>) {
        if self.frames.contains_key(&frame_id) {
            return;
        }
        if let Some(parent_frame_id) = parent_frame_id {
            if let Some(parent_frame) = self.frames.get_mut(&parent_frame_id) {
                let frame = Frame::with_parent(frame_id.clone(), parent_frame);
                self.frames.insert(frame_id, frame);
            }
        }
    }

    pub fn on_frame_detached(&mut self, event: &EventFrameDetached) {
        self.remove_frames_recursively(&event.frame_id);
    }

    pub fn on_frame_navigated(&mut self, frame: &CdpFrame) {
        if frame.parent_id.is_some() {
            if let Some((id, mut f)) = self.frames.remove_entry(&frame.id) {
                for child in f.child_frames.drain() {
                    self.remove_frames_recursively(&child);
                }
                f.navigated(frame);
                self.frames.insert(id, f);
            }
        } else {
            let mut f = if let Some(main) = self.main_frame.take() {
                // update main frame
                if let Some(mut main_frame) = self.frames.remove(&main) {
                    for child in &main_frame.child_frames {
                        self.remove_frames_recursively(child);
                    }
                    // this is necessary since we can't borrow mut and then remove recursively
                    main_frame.child_frames.clear();
                    main_frame.id = frame.id.clone();
                    main_frame
                } else {
                    Frame::new(frame.id.clone())
                }
            } else {
                // initial main frame navigation
                Frame::new(frame.id.clone())
            };
            f.navigated(frame);
            self.main_frame = Some(f.id.clone());
            self.frames.insert(f.id.clone(), f);
        }
    }

    pub fn on_frame_navigated_within_document(&mut self, event: &EventNavigatedWithinDocument) {
        if let Some(frame) = self.frames.get_mut(&event.frame_id) {
            frame.navigated_within_url(event.url.clone());
        }
        if let Some((watcher, _)) = self.navigation.as_mut() {
            watcher.on_frame_navigated_within_document(event);
        }
    }

    pub fn on_frame_stopped_loading(&mut self, event: &EventFrameStoppedLoading) {
        if let Some(frame) = self.frames.get_mut(&event.frame_id) {
            frame.on_loading_stopped();
        }
    }

    /// Fired when frame has started loading.
    pub fn on_frame_started_loading(&mut self, event: &EventFrameStartedLoading) {
        if let Some(frame) = self.frames.get_mut(&event.frame_id) {
            frame.on_loading_started();
        }
    }

    /// Notification is issued every time when binding is called
    pub fn on_runtime_binding_called(&mut self, _ev: &EventBindingCalled) {}

    /// Issued when new execution context is created
    pub fn on_frame_execution_context_created(&mut self, event: &EventExecutionContextCreated) {
        if let Some(frame_id) = event
            .context
            .aux_data
            .as_ref()
            .and_then(|v| v["frameId"].as_str())
        {
            if let Some(frame) = self.frames.get_mut(frame_id) {
                if event
                    .context
                    .aux_data
                    .as_ref()
                    .and_then(|v| v["isDefault"].as_bool())
                    .unwrap_or_default()
                {
                    frame
                        .main_world
                        .set_context(event.context.id, event.context.unique_id.clone());
                } else if event.context.name == frame.isolated_world_name
                    && frame.secondary_world.execution_context().is_none()
                {
                    frame
                        .secondary_world
                        .set_context(event.context.id, event.context.unique_id.clone());
                }
                self.context_ids
                    .insert(event.context.unique_id.clone(), frame.id.clone());
            }
        }
        if event
            .context
            .aux_data
            .as_ref()
            .filter(|v| v["type"].as_str() == Some("isolated"))
            .is_some()
        {
            self.isolated_worlds.insert(event.context.name.clone());
        }
    }

    /// Issued when execution context is destroyed
    pub fn on_frame_execution_context_destroyed(&mut self, event: &EventExecutionContextDestroyed) {
        if let Some(id) = self.context_ids.remove(&event.execution_context_unique_id) {
            if let Some(frame) = self.frames.get_mut(&id) {
                frame.destroy_context(&event.execution_context_unique_id);
            }
        }
    }

    /// Issued when all executionContexts were cleared
    pub fn on_execution_contexts_cleared(&mut self) {
        for id in self.context_ids.values() {
            if let Some(frame) = self.frames.get_mut(id) {
                frame.clear_contexts();
            }
        }
        self.context_ids.clear()
    }

    /// Fired for top level page lifecycle events (nav, load, paint, etc.)
    pub fn on_page_lifecycle_event(&mut self, event: &EventLifecycleEvent) {
        if let Some(frame) = self.frames.get_mut(&event.frame_id) {
            if event.name == "init" {
                frame.loader_id = Some(event.loader_id.clone());
                frame.lifecycle_events.clear();
            }
            frame.lifecycle_events.insert(event.name.clone().into());
        }
    }

    /// Detach all child frames
    fn remove_frames_recursively(&mut self, id: &FrameId) -> Option<Frame> {
        if let Some(mut frame) = self.frames.remove(id) {
            for child in &frame.child_frames {
                self.remove_frames_recursively(child);
            }
            if let Some(parent_id) = frame.parent_frame.take() {
                if let Some(parent) = self.frames.get_mut(&parent_id) {
                    parent.child_frames.remove(&frame.id);
                }
            }
            Some(frame)
        } else {
            None
        }
    }

    pub fn ensure_isolated_world(&mut self, world_name: &str) -> Option<CommandChain> {
        if self.isolated_worlds.contains(world_name) {
            return None;
        }

        self.isolated_worlds.insert(world_name.to_string());

        let cmd = AddScriptToEvaluateOnNewDocumentParams::builder()
            .source(format!("//# sourceURL={EVALUATION_SCRIPT_URL}"))
            .world_name(world_name)
            .build()
            .unwrap();

        let mut cmds = Vec::with_capacity(self.frames.len() + 1);

        cmds.push((cmd.identifier(), serde_json::to_value(cmd).unwrap()));

        let cm = self.frames.keys().filter_map(|id| {
            if let Ok(cmd) = CreateIsolatedWorldParams::builder()
                .frame_id(id.clone())
                .grant_univeral_access(true)
                .world_name(world_name)
                .build()
            {
                let cm = (
                    cmd.identifier(),
                    serde_json::to_value(cmd).unwrap_or_default(),
                );

                Some(cm)
            } else {
                None
            }
        });

        cmds.extend(cm);

        Some(CommandChain::new(cmds, self.request_timeout))
    }
}

#[derive(Debug)]
pub enum FrameEvent {
    /// A previously submitted navigation has finished
    NavigationResult(Result<NavigationOk, NavigationError>),
    /// A new navigation request needs to be submitted
    NavigationRequest(NavigationId, Request),
    /* /// The initial page of the target has been loaded
     * InitialPageLoadFinished */
}

#[derive(Debug)]
pub enum NavigationError {
    Timeout {
        id: NavigationId,
        err: DeadlineExceeded,
    },
    FrameNotFound {
        id: NavigationId,
        frame: FrameId,
    },
}

impl NavigationError {
    pub fn navigation_id(&self) -> &NavigationId {
        match self {
            NavigationError::Timeout { id, .. } => id,
            NavigationError::FrameNotFound { id, .. } => id,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum NavigationOk {
    SameDocumentNavigation(NavigationId),
    NewDocumentNavigation(NavigationId),
}

impl NavigationOk {
    pub fn navigation_id(&self) -> &NavigationId {
        match self {
            NavigationOk::SameDocumentNavigation(id) => id,
            NavigationOk::NewDocumentNavigation(id) => id,
        }
    }
}

/// Tracks the progress of an issued `Page.navigate` request until completion.
#[derive(Debug)]
pub struct NavigationWatcher {
    id: NavigationId,
    expected_lifecycle: HashSet<MethodId>,
    frame_id: FrameId,
    loader_id: Option<LoaderId>,
    /// Once we receive the response to the issued `Page.navigate` request we
    /// can detect whether we were navigating withing the same document or were
    /// navigating to a new document by checking if a loader was included in the
    /// response.
    same_document_navigation: bool,
}

impl NavigationWatcher {
    pub fn until_page_load(id: NavigationId, frame: FrameId, loader_id: Option<LoaderId>) -> Self {
        Self {
            id,
            expected_lifecycle: std::iter::once("load".into()).collect(),
            loader_id,
            frame_id: frame,
            same_document_navigation: false,
        }
    }

    /// Checks whether the navigation was completed
    pub fn is_lifecycle_complete(&self) -> bool {
        self.expected_lifecycle.is_empty()
    }

    fn on_frame_navigated_within_document(&mut self, ev: &EventNavigatedWithinDocument) {
        if self.frame_id == ev.frame_id {
            self.same_document_navigation = true;
        }
    }
}

/// An identifier for an ongoing navigation
#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct NavigationId(pub usize);

/// Represents a the request for a navigation
#[derive(Debug)]
pub struct FrameRequestedNavigation {
    /// The internal identifier
    pub id: NavigationId,
    /// the cdp request that will trigger the navigation
    pub req: Request,
    /// The timeout after which the request will be considered timed out
    pub timeout: Duration,
}

impl FrameRequestedNavigation {
    pub fn new(id: NavigationId, req: Request) -> Self {
        Self {
            id,
            req,
            timeout: Duration::from_millis(REQUEST_TIMEOUT),
        }
    }

    /// This will set the id of the frame into the `params` `frameId` field.
    pub fn set_frame_id(&mut self, frame_id: FrameId) {
        if let Some(params) = self.req.params.as_object_mut() {
            if let Entry::Vacant(entry) = params.entry("frameId") {
                entry.insert(serde_json::Value::String(frame_id.into()));
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LifecycleEvent {
    #[default]
    Load,
    DomcontentLoaded,
    NetworkIdle,
    NetworkAlmostIdle,
}

impl AsRef<str> for LifecycleEvent {
    fn as_ref(&self) -> &str {
        match self {
            LifecycleEvent::Load => "load",
            LifecycleEvent::DomcontentLoaded => "DOMContentLoaded",
            LifecycleEvent::NetworkIdle => "networkIdle",
            LifecycleEvent::NetworkAlmostIdle => "networkAlmostIdle",
        }
    }
}
