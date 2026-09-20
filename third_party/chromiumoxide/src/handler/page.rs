use std::sync::Arc;

use futures::channel::mpsc::{Receiver, Sender, channel, unbounded};
use futures::channel::oneshot::channel as oneshot_channel;
use futures::stream::Fuse;
use futures::{SinkExt, StreamExt};

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chromiumoxide_cdp::cdp::browser_protocol::browser::{GetVersionParams, GetVersionReturns};
use chromiumoxide_cdp::cdp::browser_protocol::dom::{
    DiscardSearchResultsParams, GetSearchResultsParams, NodeId, PerformSearchParams,
    QuerySelectorAllParams, QuerySelectorParams, Rgba,
};
use chromiumoxide_cdp::cdp::browser_protocol::emulation::{
    ClearDeviceMetricsOverrideParams, SetDefaultBackgroundColorOverrideParams,
    SetDeviceMetricsOverrideParams,
};
use chromiumoxide_cdp::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    MouseButton,
};
use chromiumoxide_cdp::cdp::browser_protocol::page::{
    AddScriptToEvaluateOnNewDocumentParams, CreateIsolatedWorldParams, FrameId,
    GetLayoutMetricsParams, GetLayoutMetricsReturns, RemoveScriptToEvaluateOnNewDocumentParams,
    Viewport,
};
use chromiumoxide_cdp::cdp::browser_protocol::target::{ActivateTargetParams, SessionId, TargetId};
use chromiumoxide_cdp::cdp::js_protocol::runtime::{
    AddBindingParams, CallFunctionOnParams, CallFunctionOnReturns, EvaluateParams,
    EventBindingCalled, ExecutionContextId, RemoteObjectId, RemoveBindingParams,
};
use chromiumoxide_types::{Command, CommandResponse};

use crate::cmd::{CommandMessage, to_command_response};
use crate::error::{CdpError, Result};
use crate::handler::RuntimeExecutionMode;
use crate::handler::commandfuture::CommandFuture;
use crate::handler::domworld::DOMWorldKind;
use crate::handler::frame::UTILITY_WORLD_NAME;
use crate::handler::httpfuture::HttpFuture;
use crate::handler::target::{CacheExecutionContext, GetExecutionContext, TargetMessage};
use crate::handler::target_message_future::TargetMessageFuture;
use crate::js::EvaluationResult;
use crate::layout::Point;
use crate::listeners::{EventListenerRequest, EventStream};
use crate::page::ScreenshotParams;
use crate::{ArcHttpRequest, keys, utils};

const ON_DEMAND_CONTEXT_ATTEMPTS: usize = 3;
const ON_DEMAND_CONTEXT_TIMEOUT: Duration = Duration::from_secs(2);
static BINDING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn temporary_binding_name() -> String {
    let sequence = BINDING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("n{timestamp:x}{sequence:x}")
}

#[derive(Debug)]
pub struct PageHandle {
    pub(crate) rx: Fuse<Receiver<TargetMessage>>,
    page: Arc<PageInner>,
}

impl PageHandle {
    pub fn new(
        target_id: TargetId,
        session_id: SessionId,
        opener_id: Option<TargetId>,
        runtime_execution_mode: RuntimeExecutionMode,
    ) -> Self {
        let (commands, rx) = channel(1);
        let page = PageInner {
            target_id,
            session_id,
            opener_id,
            sender: commands,
            runtime_execution_mode,
        };
        Self {
            rx: rx.fuse(),
            page: Arc::new(page),
        }
    }

    pub(crate) fn inner(&self) -> &Arc<PageInner> {
        &self.page
    }
}

#[derive(Debug)]
pub(crate) struct PageInner {
    target_id: TargetId,
    session_id: SessionId,
    opener_id: Option<TargetId>,
    sender: Sender<TargetMessage>,
    runtime_execution_mode: RuntimeExecutionMode,
}

impl PageInner {
    /// Execute a PDL command and return its response
    pub(crate) async fn execute<T: Command>(&self, cmd: T) -> Result<CommandResponse<T::Response>> {
        execute(cmd, self.sender.clone(), Some(self.session_id.clone())).await
    }

    /// Create a PDL command future
    pub(crate) fn command_future<T: Command>(&self, cmd: T) -> Result<CommandFuture<T>> {
        CommandFuture::new(cmd, self.sender.clone(), Some(self.session_id.clone()))
    }

    /// This creates navigation future with the final http response when the page is loaded
    pub(crate) fn wait_for_navigation(&self) -> TargetMessageFuture<ArcHttpRequest> {
        TargetMessageFuture::<ArcHttpRequest>::wait_for_navigation(self.sender.clone())
    }

    /// This creates HTTP future with navigation and responds with the final
    /// http response when the page is loaded
    pub(crate) fn http_future<T: Command>(&self, cmd: T) -> Result<HttpFuture<T>> {
        Ok(HttpFuture::new(
            self.sender.clone(),
            self.command_future(cmd)?,
        ))
    }

    /// The identifier of this page's target
    pub fn target_id(&self) -> &TargetId {
        &self.target_id
    }

    /// The identifier of this page's target's session
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// The identifier of this page's target's opener target
    pub fn opener_id(&self) -> &Option<TargetId> {
        &self.opener_id
    }

    pub(crate) fn sender(&self) -> &Sender<TargetMessage> {
        &self.sender
    }

    /// Returns the first element in the node which matches the given CSS
    /// selector.
    pub async fn find_element(&self, selector: impl Into<String>, node: NodeId) -> Result<NodeId> {
        Ok(self
            .execute(QuerySelectorParams::new(node, selector))
            .await?
            .node_id)
    }

    /// Activates (focuses) the target.
    pub async fn activate(&self) -> Result<&Self> {
        self.execute(ActivateTargetParams::new(self.target_id().clone()))
            .await?;
        Ok(self)
    }

    /// Version information about the browser
    pub async fn version(&self) -> Result<GetVersionReturns> {
        Ok(self.execute(GetVersionParams::default()).await?.result)
    }

    /// Return all `Element`s inside the node that match the given selector
    pub(crate) async fn find_elements(
        &self,
        selector: impl Into<String>,
        node: NodeId,
    ) -> Result<Vec<NodeId>> {
        Ok(self
            .execute(QuerySelectorAllParams::new(node, selector))
            .await?
            .result
            .node_ids)
    }

    /// Returns all elements which matches the given xpath selector
    pub async fn find_xpaths(&self, query: impl Into<String>) -> Result<Vec<NodeId>> {
        let perform_search_returns = self
            .execute(PerformSearchParams {
                query: query.into(),
                include_user_agent_shadow_dom: Some(true),
            })
            .await?
            .result;

        let search_results = self
            .execute(GetSearchResultsParams::new(
                perform_search_returns.search_id.clone(),
                0,
                perform_search_returns.result_count,
            ))
            .await?
            .result;

        self.execute(DiscardSearchResultsParams::new(
            perform_search_returns.search_id,
        ))
        .await?;

        Ok(search_results.node_ids)
    }

    /// Moves the mouse to this point (dispatches a mouseMoved event)
    pub async fn move_mouse(&self, point: Point) -> Result<&Self> {
        self.execute(DispatchMouseEventParams::new(
            DispatchMouseEventType::MouseMoved,
            point.x,
            point.y,
        ))
        .await?;
        Ok(self)
    }

    /// Performs a mouse click event at the point's location
    pub async fn click(&self, point: Point) -> Result<&Self> {
        let default_opts = chromiumoxide_types::ClickOptions::default();
        self.click_with(point, default_opts).await
    }

    /// Performs a mouse click event at the point's location with custom options
    pub async fn click_with(
        &self,
        point: Point,
        options: chromiumoxide_types::ClickOptions,
    ) -> Result<&Self> {
        let cmd = DispatchMouseEventParams::builder()
            .x(point.x)
            .y(point.y)
            .button(MouseButton::Left)
            .click_count(options.click_count);

        self.move_mouse(point)
            .await?
            .execute(
                cmd.clone()
                    .r#type(DispatchMouseEventType::MousePressed)
                    .build()
                    .unwrap(),
            )
            .await?;

        self.execute(
            cmd.r#type(DispatchMouseEventType::MouseReleased)
                .build()
                .unwrap(),
        )
        .await?;
        Ok(self)
    }

    /// This simulates pressing keys on the page.
    ///
    /// # Note The `input` is treated as series of `KeyDefinition`s, where each
    /// char is inserted as a separate keystroke. So sending
    /// `page.type_str("Enter")` will be processed as a series of single
    /// keystrokes:  `["E", "n", "t", "e", "r"]`. To simulate pressing the
    /// actual Enter key instead use `page.press_key(
    /// keys::get_key_definition("Enter").unwrap())`.
    pub async fn type_str(&self, input: impl AsRef<str>) -> Result<&Self> {
        for c in input.as_ref().split("").filter(|s| !s.is_empty()) {
            self.press_key(c).await?;
        }
        Ok(self)
    }

    /// Uses the `DispatchKeyEvent` mechanism to simulate pressing keyboard
    /// keys.
    pub async fn press_key(&self, key: impl AsRef<str>) -> Result<&Self> {
        let key = key.as_ref();
        let key_definition = keys::get_key_definition(key)
            .ok_or_else(|| CdpError::msg(format!("Key not found: {key}")))?;
        let mut cmd = DispatchKeyEventParams::builder();

        // See https://github.com/GoogleChrome/puppeteer/blob/62da2366c65b335751896afbb0206f23c61436f1/lib/Input.js#L114-L115
        // And https://github.com/GoogleChrome/puppeteer/blob/62da2366c65b335751896afbb0206f23c61436f1/lib/Input.js#L52
        let key_down_event_type = if let Some(txt) = key_definition.text {
            cmd = cmd.text(txt);
            DispatchKeyEventType::KeyDown
        } else if key_definition.key.len() == 1 {
            cmd = cmd.text(key_definition.key);
            DispatchKeyEventType::KeyDown
        } else {
            DispatchKeyEventType::RawKeyDown
        };

        cmd = cmd
            .r#type(DispatchKeyEventType::KeyDown)
            .key(key_definition.key)
            .code(key_definition.code)
            .windows_virtual_key_code(key_definition.key_code)
            .native_virtual_key_code(key_definition.key_code);

        self.execute(cmd.clone().r#type(key_down_event_type).build().unwrap())
            .await?;
        self.execute(cmd.r#type(DispatchKeyEventType::KeyUp).build().unwrap())
            .await?;
        Ok(self)
    }

    /// Calls function with given declaration on the remote object with the
    /// matching id
    pub async fn call_js_fn(
        &self,
        function_declaration: impl Into<String>,
        await_promise: bool,
        remote_object_id: RemoteObjectId,
    ) -> Result<CallFunctionOnReturns> {
        let resp = self
            .execute(
                CallFunctionOnParams::builder()
                    .object_id(remote_object_id)
                    .function_declaration(function_declaration)
                    .generate_preview(true)
                    .await_promise(await_promise)
                    .build()
                    .unwrap(),
            )
            .await?;
        Ok(resp.result)
    }

    pub async fn evaluate_expression(
        &self,
        evaluate: impl Into<EvaluateParams>,
    ) -> Result<EvaluationResult> {
        let mut evaluate = evaluate.into();
        if evaluate.context_id.is_none()
            && self.runtime_execution_mode == RuntimeExecutionMode::Persistent
        {
            evaluate.context_id = self.execution_context().await?;
        }
        if evaluate.await_promise.is_none() {
            evaluate.await_promise = Some(true);
        }
        if evaluate.return_by_value.is_none() {
            evaluate.return_by_value = Some(true);
        }

        let resp = self.execute(evaluate).await?.result;
        if let Some(exception) = resp.exception_details {
            return Err(CdpError::JavascriptException(Box::new(exception)));
        }

        Ok(EvaluationResult::new(resp.result))
    }

    pub async fn evaluate_function(
        &self,
        evaluate: impl Into<CallFunctionOnParams>,
    ) -> Result<EvaluationResult> {
        let mut evaluate = evaluate.into();
        if evaluate.execution_context_id.is_none() {
            evaluate.execution_context_id = self.execution_context().await?;
        }
        if evaluate.await_promise.is_none() {
            evaluate.await_promise = Some(true);
        }
        if evaluate.return_by_value.is_none() {
            evaluate.return_by_value = Some(true);
        }

        let resp = self.execute(evaluate).await?.result;
        if let Some(exception) = resp.exception_details {
            return Err(CdpError::JavascriptException(Box::new(exception)));
        }
        Ok(EvaluationResult::new(resp.result))
    }

    pub async fn execution_context(&self) -> Result<Option<ExecutionContextId>> {
        self.execution_context_for_world(None, DOMWorldKind::Main)
            .await
    }

    pub async fn secondary_execution_context(&self) -> Result<Option<ExecutionContextId>> {
        self.execution_context_for_world(None, DOMWorldKind::Secondary)
            .await
    }

    pub async fn frame_execution_context(
        &self,
        frame_id: FrameId,
    ) -> Result<Option<ExecutionContextId>> {
        self.execution_context_for_world(Some(frame_id), DOMWorldKind::Main)
            .await
    }

    pub async fn frame_secondary_execution_context(
        &self,
        frame_id: FrameId,
    ) -> Result<Option<ExecutionContextId>> {
        self.execution_context_for_world(Some(frame_id), DOMWorldKind::Secondary)
            .await
    }

    pub async fn execution_context_for_world(
        &self,
        frame_id: Option<FrameId>,
        dom_world: DOMWorldKind,
    ) -> Result<Option<ExecutionContextId>> {
        let cached = self
            .cached_execution_context(frame_id.clone(), dom_world)
            .await?;
        if cached.is_some() || self.runtime_execution_mode == RuntimeExecutionMode::Persistent {
            return Ok(cached);
        }

        let mut last_error = None;
        for attempt in 1..=ON_DEMAND_CONTEXT_ATTEMPTS {
            let resolved_frame_id = match self.resolve_frame_id(frame_id.clone()).await {
                Ok(frame_id) => frame_id,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            let result = match dom_world {
                DOMWorldKind::Main => self.acquire_main_world_context(&resolved_frame_id).await,
                DOMWorldKind::Secondary => {
                    self.acquire_isolated_world_context(&resolved_frame_id)
                        .await
                }
            };
            match result {
                Ok(execution_context_id) => {
                    self.sender
                        .clone()
                        .send(TargetMessage::CacheExecutionContext(
                            CacheExecutionContext {
                                dom_world,
                                frame_id: resolved_frame_id,
                                execution_context_id,
                            },
                        ))
                        .await?;
                    return Ok(Some(execution_context_id));
                }
                Err(error) => {
                    tracing::debug!(
                        attempt,
                        max_attempts = ON_DEMAND_CONTEXT_ATTEMPTS,
                        %error,
                        "on-demand execution context acquisition failed"
                    );
                    last_error = Some(error);
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| CdpError::msg("on-demand execution context acquisition failed")))
    }

    async fn cached_execution_context(
        &self,
        frame_id: Option<FrameId>,
        dom_world: DOMWorldKind,
    ) -> Result<Option<ExecutionContextId>> {
        let (tx, rx) = oneshot_channel();
        self.sender
            .clone()
            .send(TargetMessage::GetExecutionContext(GetExecutionContext {
                dom_world,
                frame_id,
                tx,
            }))
            .await?;
        Ok(rx.await?)
    }

    async fn resolve_frame_id(&self, frame_id: Option<FrameId>) -> Result<FrameId> {
        if let Some(frame_id) = frame_id {
            return Ok(frame_id);
        }
        let (tx, rx) = oneshot_channel();
        self.sender
            .clone()
            .send(TargetMessage::MainFrame(tx))
            .await?;
        rx.await?.ok_or(CdpError::NotFound)
    }

    async fn acquire_isolated_world_context(
        &self,
        frame_id: &FrameId,
    ) -> Result<ExecutionContextId> {
        let params = CreateIsolatedWorldParams::builder()
            .frame_id(frame_id.clone())
            .world_name(UTILITY_WORLD_NAME)
            .grant_univeral_access(true)
            .build()
            .map_err(CdpError::msg)?;
        Ok(self.execute(params).await?.result.execution_context_id)
    }

    async fn acquire_main_world_context(&self, frame_id: &FrameId) -> Result<ExecutionContextId> {
        let binding_name = temporary_binding_name();
        let payload = frame_id.inner().to_string();
        let binding_json = serde_json::to_string(&binding_name)?;
        let payload_json = serde_json::to_string(&payload)?;

        let (listener_tx, listener_rx) = unbounded();
        let mut listener = EventStream::<EventBindingCalled>::new(listener_rx);
        self.sender
            .clone()
            .send(TargetMessage::AddEventListener(
                EventListenerRequest::new::<EventBindingCalled>(listener_tx),
            ))
            .await?;
        self.execute(AddBindingParams::new(binding_name.clone()))
            .await?;

        let mut script_identifier = None;
        let mut isolated_context = None;
        let result = async {
            let source = format!(
                "(() => {{ const n = {binding_json}; const p = {payload_json}; \
                 const h = e => {{ if (e.detail === p) self[n](p); }}; \
                 document.addEventListener(n, h, {{ once: true }}); \
                 setTimeout(() => document.removeEventListener(n, h), 2000); }})()"
            );
            let script = AddScriptToEvaluateOnNewDocumentParams::builder()
                .source(source)
                .run_immediately(true)
                .build()
                .map_err(CdpError::msg)?;
            script_identifier = Some(self.execute(script).await?.result.identifier);

            let world = CreateIsolatedWorldParams::builder()
                .frame_id(frame_id.clone())
                .world_name(binding_name.clone())
                .grant_univeral_access(true)
                .build()
                .map_err(CdpError::msg)?;
            let context_id = self.execute(world).await?.result.execution_context_id;
            isolated_context = Some(context_id);

            let dispatch = EvaluateParams::builder()
                .expression(format!(
                    "document.dispatchEvent(new CustomEvent({binding_json}, {{ detail: {payload_json} }}))"
                ))
                .context_id(context_id)
                .return_by_value(true)
                .build()
                .map_err(CdpError::msg)?;
            self.execute(dispatch).await?;

            tokio::time::timeout(ON_DEMAND_CONTEXT_TIMEOUT, async {
                while let Some(event) = listener.next().await {
                    if event.name == binding_name && event.payload == payload {
                        return Ok(event.execution_context_id);
                    }
                }
                Err(CdpError::msg("Runtime.bindingCalled listener closed"))
            })
            .await
            .map_err(|_| CdpError::Timeout)?
        }
        .await;

        if let Some(identifier) = script_identifier {
            let _ = self
                .execute(RemoveScriptToEvaluateOnNewDocumentParams::new(identifier))
                .await;
        }
        if let Ok(context_id) = result.as_ref() {
            let cleanup = EvaluateParams::builder()
                .expression(format!("delete self[{binding_json}]"))
                .context_id(*context_id)
                .return_by_value(true)
                .build()
                .map_err(CdpError::msg)?;
            let _ = self.execute(cleanup).await;
        }
        if let Some(context_id) = isolated_context {
            let cleanup = EvaluateParams::builder()
                .expression(format!("delete self[{binding_json}]"))
                .context_id(context_id)
                .return_by_value(true)
                .build()
                .map_err(CdpError::msg)?;
            let _ = self.execute(cleanup).await;
        }
        let _ = self.execute(RemoveBindingParams::new(binding_name)).await;

        result
    }

    /// Returns metrics relating to the layout of the page
    pub async fn layout_metrics(&self) -> Result<GetLayoutMetricsReturns> {
        Ok(self
            .execute(GetLayoutMetricsParams::default())
            .await?
            .result)
    }

    pub async fn screenshot(&self, params: impl Into<ScreenshotParams>) -> Result<Vec<u8>> {
        self.activate().await?;
        let params = params.into();
        let full_page = params.full_page();
        let omit_background = params.omit_background();

        let mut cdp_params = params.cdp_params;

        if full_page {
            let metrics = self.layout_metrics().await?;
            let width = metrics.css_content_size.width;
            let height = metrics.css_content_size.height;

            cdp_params.clip = Some(Viewport {
                x: 0.,
                y: 0.,
                width,
                height,
                scale: 1.,
            });

            self.execute(SetDeviceMetricsOverrideParams::new(
                width as i64,
                height as i64,
                1.,
                false,
            ))
            .await?;
        }

        if omit_background {
            self.execute(SetDefaultBackgroundColorOverrideParams {
                color: Some(Rgba {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: Some(0.),
                }),
            })
            .await?;
        }

        let res = self.execute(cdp_params).await?.result;

        if omit_background {
            self.execute(SetDefaultBackgroundColorOverrideParams { color: None })
                .await?;
        }

        if full_page {
            self.execute(ClearDeviceMetricsOverrideParams {}).await?;
        }

        Ok(utils::base64::decode(&res.data)?)
    }
}

pub(crate) async fn execute<T: Command>(
    cmd: T,
    mut sender: Sender<TargetMessage>,
    session: Option<SessionId>,
) -> Result<CommandResponse<T::Response>> {
    let (tx, rx) = oneshot_channel();
    let method = cmd.identifier();
    let msg = CommandMessage::with_session(cmd, tx, session)?;

    sender.send(TargetMessage::Command(msg)).await?;
    let resp = rx.await??;
    to_command_response::<T>(resp, method)
}
