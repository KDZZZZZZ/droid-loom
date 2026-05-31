use crate::error::{AgentCoreError, AgentCoreResult};
use crate::hook::{
    HookEventRequest, HookKind, HookName, HookPayload, PointHookDecision, WrapperRequest,
    WrapperResult,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

pub type HandlerId = String;

pub trait PointHandler: Send + Sync {
    fn handle(&self, payload: HookPayload) -> AgentCoreResult<PointHookDecision>;
}

impl<F> PointHandler for F
where
    F: Fn(HookPayload) -> AgentCoreResult<PointHookDecision> + Send + Sync,
{
    fn handle(&self, payload: HookPayload) -> AgentCoreResult<PointHookDecision> {
        self(payload)
    }
}

pub trait WrapperService: Send + Sync {
    fn call(&self, request: WrapperRequest) -> AgentCoreResult<WrapperResult>;
}

impl<F> WrapperService for F
where
    F: Fn(WrapperRequest) -> AgentCoreResult<WrapperResult> + Send + Sync,
{
    fn call(&self, request: WrapperRequest) -> AgentCoreResult<WrapperResult> {
        self(request)
    }
}

pub trait WrapperLayer: Send + Sync {
    fn layer(&self, inner: Arc<dyn WrapperService>) -> Arc<dyn WrapperService>;
}

pub struct FnWrapperLayer {
    handler:
        Arc<dyn Fn(WrapperRequest, WrapperNext) -> AgentCoreResult<WrapperResult> + Send + Sync>,
}

impl FnWrapperLayer {
    pub fn new<F>(handler: F) -> Self
    where
        F: Fn(WrapperRequest, WrapperNext) -> AgentCoreResult<WrapperResult>
            + Send
            + Sync
            + 'static,
    {
        Self {
            handler: Arc::new(handler),
        }
    }
}

impl WrapperLayer for FnWrapperLayer {
    fn layer(&self, inner: Arc<dyn WrapperService>) -> Arc<dyn WrapperService> {
        let handler = Arc::clone(&self.handler);
        Arc::new(move |request| {
            let next = WrapperNext::new(Arc::clone(&inner));
            handler(request, next)
        })
    }
}

#[derive(Clone)]
pub struct WrapperNext {
    inner: Arc<dyn WrapperService>,
}

impl WrapperNext {
    pub fn new(inner: Arc<dyn WrapperService>) -> Self {
        Self { inner }
    }

    pub fn run(&self, request: WrapperRequest) -> AgentCoreResult<WrapperResult> {
        self.inner.call(request)
    }
}

impl fmt::Debug for WrapperNext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WrapperNext")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandlerRegistration {
    pub id: HandlerId,
    pub hook: HookName,
    pub kind: HandlerKind,
    pub order: i32,
    pub scope: HandlerScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandlerKind {
    Point,
    Wrapper,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandlerScope {
    CorePreGuard,
    Extension,
    Definition,
    Run,
    CorePostGuard,
}

impl Default for HandlerScope {
    fn default() -> Self {
        Self::Extension
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointHookOutcome {
    pub payload: HookPayload,
    pub status: PointHookStatus,
    pub events: Vec<HookEventRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum PointHookStatus {
    Continued,
    Blocked { reason: String },
    Stopped { reason: String },
}

#[derive(Default)]
pub struct HandlerRegistry {
    point_handlers: Vec<PointHandlerEntry>,
    wrapper_handlers: Vec<WrapperHandlerEntry>,
    next_id: usize,
}

impl HandlerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_point<F>(
        &mut self,
        hook: HookName,
        order: i32,
        handler: F,
    ) -> AgentCoreResult<HandlerRegistration>
    where
        F: PointHandler + 'static,
    {
        self.register_point_scoped(hook, order, HandlerScope::Extension, handler)
    }

    pub fn register_point_scoped<F>(
        &mut self,
        hook: HookName,
        order: i32,
        scope: HandlerScope,
        handler: F,
    ) -> AgentCoreResult<HandlerRegistration>
    where
        F: PointHandler + 'static,
    {
        if hook.kind() != HookKind::Point {
            return Err(AgentCoreError::InvalidConfig(format!(
                "hook {hook:?} is not a point hook"
            )));
        }

        let registration = self.next_registration(hook, HandlerKind::Point, order, scope);
        self.point_handlers.push(PointHandlerEntry {
            registration: registration.clone(),
            handler: Arc::new(handler),
        });
        self.sort_handlers();
        Ok(registration)
    }

    pub fn register_wrapper<F>(
        &mut self,
        hook: HookName,
        order: i32,
        handler: F,
    ) -> AgentCoreResult<HandlerRegistration>
    where
        F: Fn(WrapperRequest, WrapperNext) -> AgentCoreResult<WrapperResult>
            + Send
            + Sync
            + 'static,
    {
        self.register_wrapper_layer(hook, order, FnWrapperLayer::new(handler))
    }

    pub fn register_wrapper_layer<L>(
        &mut self,
        hook: HookName,
        order: i32,
        layer: L,
    ) -> AgentCoreResult<HandlerRegistration>
    where
        L: WrapperLayer + 'static,
    {
        self.register_wrapper_layer_scoped(hook, order, HandlerScope::Extension, layer)
    }

    pub fn register_wrapper_layer_scoped<L>(
        &mut self,
        hook: HookName,
        order: i32,
        scope: HandlerScope,
        layer: L,
    ) -> AgentCoreResult<HandlerRegistration>
    where
        L: WrapperLayer + 'static,
    {
        if hook.kind() != HookKind::Wrapper {
            return Err(AgentCoreError::InvalidConfig(format!(
                "hook {hook:?} is not a wrapper hook"
            )));
        }

        let registration = self.next_registration(hook, HandlerKind::Wrapper, order, scope);
        self.wrapper_handlers.push(WrapperHandlerEntry {
            registration: registration.clone(),
            layer: Arc::new(layer),
        });
        self.sort_handlers();
        Ok(registration)
    }

    pub fn run_point(&self, payload: HookPayload) -> AgentCoreResult<PointHookOutcome> {
        if payload.hook.kind() != HookKind::Point {
            return Err(AgentCoreError::InvalidInput(format!(
                "hook {:?} is not a point hook",
                payload.hook
            )));
        }

        let hook = payload.hook.clone();
        let mut current_payload = payload;
        let mut events = Vec::new();

        for entry in self
            .point_handlers
            .iter()
            .filter(|entry| entry.registration.hook == hook)
        {
            match entry.handler.handle(current_payload.clone())? {
                PointHookDecision::Continue => {}
                PointHookDecision::Rewrite(next_payload) => {
                    if next_payload.hook != hook {
                        return Err(AgentCoreError::InvalidInput(
                            "point handler cannot rewrite hook name".to_string(),
                        ));
                    }
                    current_payload = next_payload;
                }
                PointHookDecision::Emit { event } => events.push(event),
                PointHookDecision::Block { reason } => {
                    return Ok(PointHookOutcome {
                        payload: current_payload,
                        status: PointHookStatus::Blocked { reason },
                        events,
                    });
                }
                PointHookDecision::Stop { reason } => {
                    return Ok(PointHookOutcome {
                        payload: current_payload,
                        status: PointHookStatus::Stopped { reason },
                        events,
                    });
                }
            }
        }

        Ok(PointHookOutcome {
            payload: current_payload,
            status: PointHookStatus::Continued,
            events,
        })
    }

    pub fn run_wrapper<F>(
        &self,
        request: WrapperRequest,
        terminal: F,
    ) -> AgentCoreResult<WrapperResult>
    where
        F: Fn(WrapperRequest) -> AgentCoreResult<WrapperResult> + Send + Sync + 'static,
    {
        if request.hook.kind() != HookKind::Wrapper {
            return Err(AgentCoreError::InvalidInput(format!(
                "hook {:?} is not a wrapper hook",
                request.hook
            )));
        }

        let service = self.wrapper_service(request.hook.clone(), Arc::new(terminal));
        service.call(request)
    }

    pub fn wrapper_service(
        &self,
        hook: HookName,
        terminal: Arc<dyn WrapperService>,
    ) -> Arc<dyn WrapperService> {
        self.wrapper_handlers
            .iter()
            .filter(|entry| entry.registration.hook == hook)
            .rev()
            .fold(terminal, |inner, entry| entry.layer.layer(inner))
    }

    fn next_registration(
        &mut self,
        hook: HookName,
        kind: HandlerKind,
        order: i32,
        scope: HandlerScope,
    ) -> HandlerRegistration {
        self.next_id += 1;
        HandlerRegistration {
            id: format!("handler-{}", self.next_id),
            hook,
            kind,
            order,
            scope,
        }
    }

    fn sort_handlers(&mut self) {
        self.point_handlers
            .sort_by(|left, right| compare_registration(&left.registration, &right.registration));
        self.wrapper_handlers
            .sort_by(|left, right| compare_registration(&left.registration, &right.registration));
    }
}

impl fmt::Debug for HandlerRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HandlerRegistry")
            .field("point_handlers", &self.point_handlers.len())
            .field("wrapper_handlers", &self.wrapper_handlers.len())
            .finish()
    }
}

#[derive(Clone)]
struct PointHandlerEntry {
    registration: HandlerRegistration,
    handler: Arc<dyn PointHandler>,
}

#[derive(Clone)]
struct WrapperHandlerEntry {
    registration: HandlerRegistration,
    layer: Arc<dyn WrapperLayer>,
}

fn compare_registration(
    left: &HandlerRegistration,
    right: &HandlerRegistration,
) -> std::cmp::Ordering {
    left.scope
        .cmp(&right.scope)
        .then_with(|| left.order.cmp(&right.order))
        .then_with(|| left.id.cmp(&right.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::{WrapperResponse, WrapperResult};
    use serde_json::json;

    #[test]
    fn point_handlers_rewrite_in_order() {
        let mut registry = HandlerRegistry::new();
        registry
            .register_point(HookName::Input, 10, |payload: HookPayload| {
                let mut payload = payload;
                payload.data = json!({"step": 2});
                Ok(PointHookDecision::Rewrite(payload))
            })
            .unwrap();
        registry
            .register_point(HookName::Input, 0, |payload: HookPayload| {
                let mut payload = payload;
                payload.data = json!({"step": 1});
                Ok(PointHookDecision::Rewrite(payload))
            })
            .unwrap();

        let outcome = registry
            .run_point(HookPayload::new(HookName::Input))
            .unwrap();

        assert_eq!(outcome.status, PointHookStatus::Continued);
        assert_eq!(outcome.payload.data, json!({"step": 2}));
    }

    #[test]
    fn point_handler_can_short_circuit_with_block() {
        let mut registry = HandlerRegistry::new();
        registry
            .register_point(HookName::ToolCall, 0, |_payload: HookPayload| {
                Ok(PointHookDecision::Block {
                    reason: "denied".to_string(),
                })
            })
            .unwrap();

        let outcome = registry
            .run_point(HookPayload::new(HookName::ToolCall))
            .unwrap();

        assert_eq!(
            outcome.status,
            PointHookStatus::Blocked {
                reason: "denied".to_string()
            }
        );
    }

    #[test]
    fn wrapper_layers_compose_around_terminal_in_order() {
        let mut registry = HandlerRegistry::new();
        registry
            .register_wrapper(
                HookName::NodeExecution,
                0,
                |mut request: WrapperRequest, next| {
                    request.metadata.insert("outer".to_string(), json!(true));
                    next.run(request)
                },
            )
            .unwrap();
        registry
            .register_wrapper(
                HookName::NodeExecution,
                10,
                |mut request: WrapperRequest, next| {
                    request.metadata.insert("inner".to_string(), json!(true));
                    next.run(request)
                },
            )
            .unwrap();

        let result = registry
            .run_wrapper(WrapperRequest::new(HookName::NodeExecution), |request| {
                Ok(WrapperResult::Continue(WrapperResponse::new(json!(
                    request.metadata
                ))))
            })
            .unwrap();

        let WrapperResult::Continue(response) = result else {
            panic!("unexpected wrapper result");
        };
        assert_eq!(response.data["outer"], json!(true));
        assert_eq!(response.data["inner"], json!(true));
    }
}
