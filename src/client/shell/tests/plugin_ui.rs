use super::*;

use crate::protocol::endpoint::{EndpointPluginUi, EndpointPluginUiItem};

fn plugin_rows() -> Vec<EndpointPluginUiItem> {
    vec![
        EndpointPluginUiItem {
            plugin_id: "example.ui".into(),
            ui_id: "review".into(),
            title: "Review focus".into(),
            action: Some("focus".into()),
        },
        EndpointPluginUiItem {
            plugin_id: "example.ui".into(),
            ui_id: "zoom".into(),
            title: "Zoom label".into(),
            action: None,
        },
    ]
}

fn state_with_plugin_rows() -> ClientShellState {
    let config = ClientShellConfig::from_config(&Config::default());
    let mut state = ClientShellState::new(config);
    state.sidebar_collapsed = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.set_endpoint_plugin_ui(
        &ClientEndpointId::Local,
        EndpointPluginUi {
            boot_id: "boot-1".into(),
            revision: 1,
            ui: plugin_rows(),
        },
    );
    state
}

#[test]
fn expanded_sidebar_renders_plugin_rows_with_hits() {
    let mut state = state_with_plugin_rows();
    let frame = state.compose(106, 20).expect("expanded sidebar");
    assert_eq!(state.hits.plugin_ui.len(), 2);
    let review = state.hits.plugin_ui[0].rect;
    assert!(review.width > 0 && review.height == 1);
    let buffer = frame.to_ratatui_buffer().expect("sidebar buffer");
    let (x, y) = cell_symbol_position(&frame, review, "Review");
    assert_eq!(buffer.cell((x, y)).expect("title cell").symbol(), "R");
}

#[test]
fn clicking_plugin_row_invokes_declared_action() {
    let mut state = state_with_plugin_rows();
    state.compose(106, 20).expect("expanded sidebar");
    let review = state.hits.plugin_ui[0].rect;
    let press = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: review.x + 1,
        row: review.y,
        modifiers: crossterm::event::KeyModifiers::empty(),
    })]);
    assert!(matches!(
        &press.actions[..],
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(
                &request.method,
                crate::api::schema::Method::PluginActionInvoke(params)
                    if params.action_id == "focus"
                        && params.plugin_id.as_deref() == Some("example.ui")
            )
    ));
}

#[test]
fn plugin_row_without_action_is_not_clickable() {
    let mut state = state_with_plugin_rows();
    state.compose(106, 20).expect("expanded sidebar");
    let zoom = state.hits.plugin_ui[1].rect;
    let press = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: zoom.x + 1,
        row: zoom.y,
        modifiers: crossterm::event::KeyModifiers::empty(),
    })]);
    assert!(!press.actions.iter().any(|action| matches!(
        action,
        ClientShellAction::Endpoint { request, .. }
            if matches!(request.method, crate::api::schema::Method::PluginActionInvoke(_))
    )));
}
