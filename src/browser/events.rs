use super::{Dom, TinyJsRuntime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BrowserClickDispatch {
    pub(super) node_id: usize,
    pub(super) default_prevented: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BrowserEventDispatch {
    pub(super) node_id: usize,
    pub(super) default_prevented: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum BrowserEventTarget {
    Window,
    Node(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct JsEventListener {
    pub(super) handler: String,
    pub(super) capture: bool,
    pub(super) once: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TinyJsEvent {
    pub(super) type_name: String,
    pub(super) target_node: usize,
    pub(super) key: Option<String>,
    pub(super) data: Option<String>,
    pub(super) input_type: Option<String>,
    pub(super) client_x: Option<usize>,
    pub(super) client_y: Option<usize>,
    pub(super) button: Option<i32>,
    pub(super) pointer_id: Option<i32>,
    pub(super) pointer_type: Option<String>,
    pub(super) is_primary: Option<bool>,
    pub(super) delta_x: Option<isize>,
    pub(super) delta_y: Option<isize>,
    pub(super) phase: BrowserEventPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BrowserEventPhase {
    Capture,
    Target,
    Bubble,
}

impl BrowserEventPhase {
    pub(super) fn as_dom_event_phase(self) -> &'static str {
        match self {
            Self::Capture => "1",
            Self::Target => "2",
            Self::Bubble => "3",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BrowserEventPayload {
    pub(super) type_name: String,
    pub(super) target_node: usize,
    pub(super) key: Option<String>,
    pub(super) data: Option<String>,
    pub(super) input_type: Option<String>,
    pub(super) client_x: Option<usize>,
    pub(super) client_y: Option<usize>,
    pub(super) button: Option<i32>,
    pub(super) pointer_id: Option<i32>,
    pub(super) pointer_type: Option<String>,
    pub(super) is_primary: Option<bool>,
    pub(super) delta_x: Option<isize>,
    pub(super) delta_y: Option<isize>,
}

impl BrowserEventPayload {
    pub(super) fn new(type_name: &str, target_node: usize) -> Self {
        Self {
            type_name: type_name.to_owned(),
            target_node,
            key: None,
            data: None,
            input_type: None,
            client_x: None,
            client_y: None,
            button: None,
            pointer_id: None,
            pointer_type: None,
            is_primary: None,
            delta_x: None,
            delta_y: None,
        }
    }

    pub(super) fn keyboard(type_name: &str, target_node: usize, key: &str) -> Self {
        Self {
            key: Some(key.to_owned()),
            ..Self::new(type_name, target_node)
        }
    }

    pub(super) fn beforeinput(target_node: usize, input_type: &str, data: Option<&str>) -> Self {
        Self {
            type_name: "beforeinput".to_owned(),
            target_node,
            key: None,
            data: data.map(str::to_owned),
            input_type: Some(input_type.to_owned()),
            client_x: None,
            client_y: None,
            button: None,
            pointer_id: None,
            pointer_type: None,
            is_primary: None,
            delta_x: None,
            delta_y: None,
        }
    }

    pub(super) fn wheel(target_node: usize, delta_x: isize, delta_y: isize) -> Self {
        Self {
            delta_x: Some(delta_x),
            delta_y: Some(delta_y),
            ..Self::new("wheel", target_node)
        }
    }

    pub(super) fn mouse(type_name: &str, target_node: usize, x: usize, y: usize) -> Self {
        Self {
            client_x: Some(x),
            client_y: Some(y),
            button: Some(0),
            ..Self::new(type_name, target_node)
        }
    }

    pub(super) fn pointer(type_name: &str, target_node: usize, x: usize, y: usize) -> Self {
        Self {
            pointer_id: Some(1),
            pointer_type: Some("mouse".to_owned()),
            is_primary: Some(true),
            ..Self::mouse(type_name, target_node, x, y)
        }
    }

    pub(super) fn into_event(self) -> TinyJsEvent {
        TinyJsEvent {
            type_name: self.type_name,
            target_node: self.target_node,
            key: self.key,
            data: self.data,
            input_type: self.input_type,
            client_x: self.client_x,
            client_y: self.client_y,
            button: self.button,
            pointer_id: self.pointer_id,
            pointer_type: self.pointer_type,
            is_primary: self.is_primary,
            delta_x: self.delta_x,
            delta_y: self.delta_y,
            phase: BrowserEventPhase::Target,
        }
    }
}

#[derive(Debug)]
pub(super) struct BrowserEventRuntimeSnapshot {
    default_prevented: bool,
    propagation_stopped: bool,
    immediate_propagation_stopped: bool,
    return_false_prevents_default: bool,
    current_event: Option<TinyJsEvent>,
}

pub(super) fn begin_click_dispatch(
    runtime: &mut TinyJsRuntime,
    payload: BrowserEventPayload,
) -> BrowserEventRuntimeSnapshot {
    let snapshot = capture_event_runtime(runtime, payload);
    runtime.default_prevented = false;
    runtime.propagation_stopped = false;
    runtime.immediate_propagation_stopped = false;
    snapshot
}

pub(super) fn begin_event_dispatch(
    runtime: &mut TinyJsRuntime,
    payload: BrowserEventPayload,
) -> BrowserEventRuntimeSnapshot {
    let snapshot = begin_click_dispatch(runtime, payload);
    runtime.return_false_prevents_default = false;
    snapshot
}

fn capture_event_runtime(
    runtime: &mut TinyJsRuntime,
    payload: BrowserEventPayload,
) -> BrowserEventRuntimeSnapshot {
    BrowserEventRuntimeSnapshot {
        default_prevented: runtime.default_prevented,
        propagation_stopped: runtime.propagation_stopped,
        immediate_propagation_stopped: runtime.immediate_propagation_stopped,
        return_false_prevents_default: runtime.return_false_prevents_default,
        current_event: runtime.current_event.replace(payload.into_event()),
    }
}

pub(super) fn restore_event_dispatch(
    runtime: &mut TinyJsRuntime,
    snapshot: BrowserEventRuntimeSnapshot,
) -> bool {
    let default_prevented = runtime.default_prevented;
    runtime.default_prevented = snapshot.default_prevented;
    runtime.propagation_stopped = snapshot.propagation_stopped;
    runtime.immediate_propagation_stopped = snapshot.immediate_propagation_stopped;
    runtime.return_false_prevents_default = snapshot.return_false_prevents_default;
    runtime.current_event = snapshot.current_event;
    default_prevented
}

pub(super) fn event_path_to_window(dom: &Dom, node_id: usize) -> Vec<BrowserEventTarget> {
    let mut path = Vec::new();
    let mut next = Some(node_id);
    while let Some(current_node_id) = next {
        if current_node_id >= dom.nodes.len() || path.len() > dom.nodes.len() {
            break;
        }
        path.push(BrowserEventTarget::Node(current_node_id));
        next = dom.nodes.get(current_node_id).and_then(|node| node.parent);
    }
    path.push(BrowserEventTarget::Window);
    path
}

pub(super) fn set_runtime_this_target(
    runtime: &mut TinyJsRuntime,
    target: BrowserEventTarget,
) -> (Option<usize>, Option<BrowserEventTarget>) {
    let previous = (runtime.this_node, runtime.this_target);
    runtime.this_target = Some(target);
    runtime.this_node = match target {
        BrowserEventTarget::Node(node_id) => Some(node_id),
        BrowserEventTarget::Window => None,
    };
    previous
}

pub(super) fn restore_runtime_this_target(
    runtime: &mut TinyJsRuntime,
    previous: (Option<usize>, Option<BrowserEventTarget>),
) {
    runtime.this_node = previous.0;
    runtime.this_target = previous.1;
}

pub(super) fn set_current_event_phase(runtime: &mut TinyJsRuntime, phase: BrowserEventPhase) {
    if let Some(event) = runtime.current_event.as_mut() {
        event.phase = phase;
    }
}

pub(super) fn dispatch_event_listener_group<F>(
    runtime: &mut TinyJsRuntime,
    target: BrowserEventTarget,
    event_name: &str,
    capture: bool,
    phase: BrowserEventPhase,
    mut execute_handler: F,
) where
    F: FnMut(&mut TinyJsRuntime, &str),
{
    let previous_this = set_runtime_this_target(runtime, target);
    set_current_event_phase(runtime, phase);
    let listener_key = (target, event_name.to_owned());
    let mut once_listener_indices = Vec::new();
    if let Some(listeners) = runtime.event_listeners.get(&listener_key).cloned() {
        for (listener_index, listener) in listeners
            .iter()
            .enumerate()
            .filter(|(_, listener)| listener.capture == capture)
        {
            execute_handler(runtime, &listener.handler);
            if listener.once {
                once_listener_indices.push(listener_index);
            }
            if runtime.immediate_propagation_stopped {
                break;
            }
        }
    }
    if !once_listener_indices.is_empty()
        && let Some(listeners) = runtime.event_listeners.get_mut(&listener_key)
    {
        let mut current_index = 0usize;
        listeners.retain(|_| {
            let keep = !once_listener_indices.contains(&current_index);
            current_index += 1;
            keep
        });
    }
    restore_runtime_this_target(runtime, previous_this);
}

#[cfg(test)]
mod tests {
    use super::super::{Node, NodeKind};
    use super::*;

    #[test]
    fn event_payload_builders_preserve_observable_fields() {
        let keyboard = BrowserEventPayload::keyboard("keydown", 7, "a").into_event();
        assert_eq!(keyboard.type_name, "keydown");
        assert_eq!(keyboard.target_node, 7);
        assert_eq!(keyboard.key.as_deref(), Some("a"));
        assert_eq!(keyboard.data, None);
        assert_eq!(keyboard.input_type, None);
        assert_eq!(keyboard.phase.as_dom_event_phase(), "2");

        let beforeinput = BrowserEventPayload::beforeinput(9, "insertText", Some("x")).into_event();
        assert_eq!(beforeinput.type_name, "beforeinput");
        assert_eq!(beforeinput.target_node, 9);
        assert_eq!(beforeinput.key, None);
        assert_eq!(beforeinput.data.as_deref(), Some("x"));
        assert_eq!(beforeinput.input_type.as_deref(), Some("insertText"));
        assert_eq!(beforeinput.phase, BrowserEventPhase::Target);

        let pointer = BrowserEventPayload::pointer("pointerdown", 11, 3, 5).into_event();
        assert_eq!(pointer.type_name, "pointerdown");
        assert_eq!(pointer.target_node, 11);
        assert_eq!(pointer.client_x, Some(3));
        assert_eq!(pointer.client_y, Some(5));
        assert_eq!(pointer.button, Some(0));
        assert_eq!(pointer.pointer_id, Some(1));
        assert_eq!(pointer.pointer_type.as_deref(), Some("mouse"));
        assert_eq!(pointer.is_primary, Some(true));

        let mouse = BrowserEventPayload::mouse("click", 11, 3, 5).into_event();
        assert_eq!(mouse.client_x, Some(3));
        assert_eq!(mouse.client_y, Some(5));
        assert_eq!(mouse.button, Some(0));
        assert_eq!(mouse.pointer_id, None);
        assert_eq!(mouse.pointer_type, None);
        assert_eq!(mouse.is_primary, None);

        let wheel = BrowserEventPayload::wheel(13, -2, 7).into_event();
        assert_eq!(wheel.type_name, "wheel");
        assert_eq!(wheel.target_node, 13);
        assert_eq!(wheel.delta_x, Some(-2));
        assert_eq!(wheel.delta_y, Some(7));
    }

    #[test]
    fn dom_event_phase_values_match_browser_constants() {
        assert_eq!(BrowserEventPhase::Capture.as_dom_event_phase(), "1");
        assert_eq!(BrowserEventPhase::Target.as_dom_event_phase(), "2");
        assert_eq!(BrowserEventPhase::Bubble.as_dom_event_phase(), "3");
    }

    #[test]
    fn event_path_walks_ancestors_to_window() {
        let dom = Dom {
            nodes: vec![
                Node {
                    kind: NodeKind::Document,
                    parent: None,
                    children: vec![1],
                },
                Node {
                    kind: NodeKind::Text("parent".to_owned()),
                    parent: Some(0),
                    children: vec![2],
                },
                Node {
                    kind: NodeKind::Text("target".to_owned()),
                    parent: Some(1),
                    children: Vec::new(),
                },
            ],
        };

        assert_eq!(
            event_path_to_window(&dom, 2),
            vec![
                BrowserEventTarget::Node(2),
                BrowserEventTarget::Node(1),
                BrowserEventTarget::Node(0),
                BrowserEventTarget::Window,
            ]
        );
        assert_eq!(
            event_path_to_window(&dom, 99),
            vec![BrowserEventTarget::Window]
        );
    }

    #[test]
    fn event_dispatch_snapshot_restores_outer_runtime_flags() {
        let mut runtime = TinyJsRuntime {
            default_prevented: true,
            propagation_stopped: true,
            immediate_propagation_stopped: true,
            return_false_prevents_default: true,
            current_event: Some(BrowserEventPayload::new("outer", 1).into_event()),
            ..TinyJsRuntime::default()
        };

        let snapshot = begin_event_dispatch(
            &mut runtime,
            BrowserEventPayload::keyboard("keydown", 2, "x"),
        );
        assert!(!runtime.default_prevented);
        assert!(!runtime.propagation_stopped);
        assert!(!runtime.immediate_propagation_stopped);
        assert!(!runtime.return_false_prevents_default);
        assert_eq!(
            runtime
                .current_event
                .as_ref()
                .map(|event| event.type_name.as_str()),
            Some("keydown")
        );

        runtime.default_prevented = true;
        assert!(restore_event_dispatch(&mut runtime, snapshot));
        assert!(runtime.default_prevented);
        assert!(runtime.propagation_stopped);
        assert!(runtime.immediate_propagation_stopped);
        assert!(runtime.return_false_prevents_default);
        assert_eq!(
            runtime
                .current_event
                .as_ref()
                .map(|event| event.type_name.as_str()),
            Some("outer")
        );
    }

    #[test]
    fn click_dispatch_snapshot_preserves_return_false_mode_until_restore() {
        let mut runtime = TinyJsRuntime {
            return_false_prevents_default: true,
            ..TinyJsRuntime::default()
        };

        let snapshot = begin_click_dispatch(&mut runtime, BrowserEventPayload::new("click", 4));
        assert!(runtime.return_false_prevents_default);

        runtime.return_false_prevents_default = false;
        assert!(!restore_event_dispatch(&mut runtime, snapshot));
        assert!(runtime.return_false_prevents_default);
    }

    #[test]
    fn listener_group_filters_capture_sets_phase_and_removes_once() {
        let mut runtime = TinyJsRuntime {
            this_node: Some(8),
            this_target: Some(BrowserEventTarget::Node(8)),
            current_event: Some(BrowserEventPayload::new("click", 4).into_event()),
            ..TinyJsRuntime::default()
        };
        runtime.event_listeners.insert(
            (BrowserEventTarget::Node(4), "click".to_owned()),
            vec![
                JsEventListener {
                    handler: "capture-once".to_owned(),
                    capture: true,
                    once: true,
                },
                JsEventListener {
                    handler: "bubble".to_owned(),
                    capture: false,
                    once: false,
                },
                JsEventListener {
                    handler: "capture".to_owned(),
                    capture: true,
                    once: false,
                },
            ],
        );

        let mut seen = Vec::new();
        dispatch_event_listener_group(
            &mut runtime,
            BrowserEventTarget::Node(4),
            "click",
            true,
            BrowserEventPhase::Capture,
            |runtime, handler| {
                seen.push(format!(
                    "{handler}:{}:{}",
                    runtime.this_node.unwrap(),
                    runtime
                        .current_event
                        .as_ref()
                        .unwrap()
                        .phase
                        .as_dom_event_phase()
                ));
            },
        );

        assert_eq!(seen, vec!["capture-once:4:1", "capture:4:1"]);
        assert_eq!(runtime.this_node, Some(8));
        assert_eq!(runtime.this_target, Some(BrowserEventTarget::Node(8)));
        let remaining = runtime
            .event_listeners
            .get(&(BrowserEventTarget::Node(4), "click".to_owned()))
            .unwrap();
        assert_eq!(
            remaining
                .iter()
                .map(|listener| listener.handler.as_str())
                .collect::<Vec<_>>(),
            vec!["bubble", "capture"]
        );
    }

    #[test]
    fn listener_group_stops_after_immediate_propagation() {
        let mut runtime = TinyJsRuntime {
            current_event: Some(BrowserEventPayload::new("click", 4).into_event()),
            ..TinyJsRuntime::default()
        };
        runtime.event_listeners.insert(
            (BrowserEventTarget::Node(4), "click".to_owned()),
            vec![
                JsEventListener {
                    handler: "first".to_owned(),
                    capture: false,
                    once: false,
                },
                JsEventListener {
                    handler: "bad-second".to_owned(),
                    capture: false,
                    once: false,
                },
            ],
        );

        let mut seen = Vec::new();
        dispatch_event_listener_group(
            &mut runtime,
            BrowserEventTarget::Node(4),
            "click",
            false,
            BrowserEventPhase::Target,
            |runtime, handler| {
                seen.push(handler.to_owned());
                runtime.immediate_propagation_stopped = true;
            },
        );

        assert_eq!(seen, vec!["first"]);
    }
}
