//! Dynamic plugin UI rows: spawning refresh commands, parsing their output,
//! and caching the resulting rows for sidebar projection.

use crate::api::schema::{PluginManifestAction, PluginUiSurface};
use crate::app::state::InstalledPluginRegistry;
use crate::protocol::endpoint::EndpointPluginUiItem;

use super::manifest::{current_platform, effective_platforms};
use super::plugin_manifest_available;
use crate::app::App;

/// Cap on rows accepted from one refresh command.
const PLUGIN_UI_MAX_ROWS: usize = 128;
/// Cap on a single row title; the client clips to the sidebar width anyway.
const PLUGIN_UI_TITLE_MAX_CHARS: usize = 200;

#[derive(serde::Deserialize)]
struct PluginUiRowJson {
    title: String,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    tab_id: Option<String>,
    #[serde(default)]
    workspace_id: Option<String>,
}

impl App {
    /// Mark every enabled dynamic sidebar entry dirty and start refreshes for
    /// the ones without a command in flight. Called when session state that a
    /// tree-style panel reflects changes, and after the registry reloads.
    pub(crate) fn refresh_dynamic_plugin_ui(&mut self) {
        let entries = dynamic_ui_entries(&self.state.installed_plugins);
        for key in entries {
            self.state.plugin_ui_refresh_dirty.insert(key);
        }
        self.pump_plugin_ui_refreshes();
    }

    /// Refresh dynamic rows after a plugin action finished, so actions that
    /// only mutate plugin-local state (for example a collapse toggle) still
    /// produce updated rows. No-op when the plugin has no dynamic entries.
    pub(crate) fn refresh_dynamic_plugin_ui_after_action(&mut self, log_id: &str) {
        let Some(plugin_id) = self
            .state
            .plugin_command_logs
            .iter()
            .find(|log| log.log_id == log_id && log.action_id.is_some())
            .map(|log| log.plugin_id.clone())
        else {
            return;
        };
        let has_dynamic = self
            .state
            .installed_plugins
            .get(&plugin_id)
            .is_some_and(|plugin| {
                plugin.enabled && plugin.ui.iter().any(|entry| entry.command.is_some())
            });
        if has_dynamic {
            self.refresh_dynamic_plugin_ui();
        }
    }

    /// Start refresh commands for dirty entries that have none in flight.
    pub(crate) fn pump_plugin_ui_refreshes(&mut self) {
        let dirty: Vec<(String, String)> =
            self.state.plugin_ui_refresh_dirty.iter().cloned().collect();
        for key in dirty {
            let (plugin_id, ui_id) = (&key.0, &key.1);
            if self
                .state
                .plugin_ui_refresh_in_flight
                .values()
                .any(|in_flight| *in_flight == key)
            {
                continue;
            }
            let Some(plugin) = self
                .state
                .installed_plugins
                .get(plugin_id.as_str())
                .cloned()
            else {
                self.state.plugin_ui_refresh_dirty.remove(&key);
                self.state.plugin_ui_dynamic.remove(&key);
                continue;
            };
            if !plugin.enabled || !plugin_manifest_available(&plugin) {
                self.state.plugin_ui_refresh_dirty.remove(&key);
                self.state.plugin_ui_dynamic.remove(&key);
                continue;
            }
            let Some(entry) = plugin.ui.iter().find(|entry| entry.id == *ui_id) else {
                self.state.plugin_ui_refresh_dirty.remove(&key);
                self.state.plugin_ui_dynamic.remove(&key);
                continue;
            };
            let Some(command) = entry.command.clone() else {
                self.state.plugin_ui_refresh_dirty.remove(&key);
                continue;
            };
            let mut context = self.current_plugin_context("plugin-ui-refresh");
            context.invocation_source = Some("plugin_ui_refresh".to_string());
            let started = self.start_plugin_command(&plugin, None, None, command, &context, None);
            self.state.plugin_ui_refresh_dirty.remove(&key);
            match started {
                Ok(log) => {
                    self.state
                        .plugin_ui_refresh_in_flight
                        .insert(log.log_id, key);
                }
                Err((code, message)) => {
                    tracing::warn!(
                        plugin_id = %plugin_id,
                        ui_id = %ui_id,
                        code,
                        "dynamic plugin ui refresh failed to start: {message}"
                    );
                }
            }
        }
    }

    /// Route a finished plugin command to the dynamic UI cache when it was a
    /// refresh command. Returns true when the rows projection changed.
    pub(crate) fn handle_plugin_ui_refresh_finished(
        &mut self,
        log_id: &str,
        exit_code: Option<i32>,
        stdout: &str,
    ) -> bool {
        let Some(key) = self.state.plugin_ui_refresh_in_flight.remove(log_id) else {
            return false;
        };
        let rows = if exit_code == Some(0) {
            let actions = self
                .state
                .installed_plugins
                .get(&key.0)
                .map(|plugin| plugin.actions.as_slice())
                .unwrap_or(&[]);
            parse_plugin_ui_rows(&key.0, &key.1, stdout, actions)
        } else {
            Vec::new()
        };
        let changed = self.state.plugin_ui_dynamic.get(&key) != Some(&rows);
        if changed {
            self.state.plugin_ui_dynamic.insert(key, rows);
        }
        changed
    }
}

/// Enabled dynamic sidebar entries (plugin_id, ui_id) for the current platform.
pub(super) fn dynamic_ui_entries(plugins: &InstalledPluginRegistry) -> Vec<(String, String)> {
    let host = current_platform();
    let mut entries = Vec::new();
    for plugin in plugins.values() {
        if !plugin.enabled || !plugin_manifest_available(plugin) {
            continue;
        }
        for entry in &plugin.ui {
            if entry.surface != PluginUiSurface::Sidebar || entry.command.is_none() {
                continue;
            }
            if let Some(platforms) = effective_platforms(&entry.platforms, &plugin.platforms) {
                if !platforms.contains(&host) {
                    continue;
                }
            }
            entries.push((plugin.plugin_id.clone(), entry.id.clone()));
        }
    }
    entries.sort();
    entries
}

/// Parse the JSON rows printed by a dynamic UI refresh command. Invalid output
/// yields no rows; row action ids that the plugin never declared are stripped
/// so the row renders but stays non-clickable.
pub(super) fn parse_plugin_ui_rows(
    plugin_id: &str,
    ui_id: &str,
    stdout: &str,
    actions: &[PluginManifestAction],
) -> Vec<EndpointPluginUiItem> {
    let parsed: Vec<PluginUiRowJson> = match serde_json::from_str(stdout) {
        Ok(parsed) => parsed,
        Err(err) => {
            tracing::warn!(
                plugin_id,
                ui_id,
                "dynamic plugin ui rows failed to parse: {err}"
            );
            return Vec::new();
        }
    };
    parsed
        .into_iter()
        .take(PLUGIN_UI_MAX_ROWS)
        .filter_map(|row| {
            let title = row.title.trim().to_string();
            if title.is_empty() {
                return None;
            }
            let title = title.chars().take(PLUGIN_UI_TITLE_MAX_CHARS).collect();
            let action = row
                .action
                .filter(|action| actions.iter().any(|declared| &declared.id == action));
            Some(EndpointPluginUiItem {
                plugin_id: plugin_id.to_string(),
                ui_id: ui_id.to_string(),
                title,
                action,
                tab_id: row.tab_id.filter(|id| !id.is_empty()),
                workspace_id: row.workspace_id.filter(|id| !id.is_empty()),
            })
        })
        .collect()
}

/// Whether a dynamic plugin ui entry benefits from a refresh after this event.
pub(crate) fn event_touches_plugin_ui(event: &crate::api::schema::EventKind) -> bool {
    use crate::api::schema::EventKind as E;
    matches!(
        event,
        E::WorkspaceCreated
            | E::WorkspaceUpdated
            | E::WorkspaceMetadataUpdated
            | E::WorkspaceClosed
            | E::WorkspaceReordered
            | E::WorkspaceRenamed
            | E::WorkspaceMoved
            | E::WorkspaceFocused
            | E::WorktreeCreated
            | E::WorktreeOpened
            | E::WorktreeRemoved
            | E::TabCreated
            | E::TabClosed
            | E::TabRenamed
            | E::TabMoved
            | E::TabFocused
            | E::PaneAgentDetected
            | E::PaneAgentStatusChanged
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(id: &str) -> PluginManifestAction {
        PluginManifestAction {
            id: id.to_string(),
            title: id.to_string(),
            description: None,
            contexts: Vec::new(),
            platforms: None,
            command: vec!["true".to_string()],
        }
    }

    fn parse(stdout: &str, actions: &[PluginManifestAction]) -> Vec<EndpointPluginUiItem> {
        parse_plugin_ui_rows("demo", "tree", stdout, actions)
    }

    #[test]
    fn parses_rows_with_context_and_declared_actions() {
        let rows = parse(
            r#"[{"title":"▾ main","action":"open","workspace_id":"ws1"},
                {"title":"  ● build","action":"focus","tab_id":"t2","workspace_id":"ws1"}]"#,
            &[action("open"), action("focus")],
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title, "▾ main");
        assert_eq!(rows[0].action.as_deref(), Some("open"));
        assert_eq!(rows[0].workspace_id.as_deref(), Some("ws1"));
        assert_eq!(rows[1].tab_id.as_deref(), Some("t2"));
        assert_eq!(rows[1].ui_id, "tree");
    }

    #[test]
    fn undeclared_row_action_is_stripped() {
        let rows = parse(r#"[{"title":"row","action":"missing"}]"#, &[action("open")]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action, None);
    }

    #[test]
    fn invalid_json_or_blank_titles_yield_no_rows() {
        assert!(parse("not json", &[action("open")]).is_empty());
        assert!(parse(r#"[{"title":"  "}]"#, &[action("open")]).is_empty());
    }

    #[test]
    fn caps_row_count_and_title_length() {
        let many: String = (0..500)
            .map(|i| format!(r#"{{"title":"row{i}"}},"#))
            .collect();
        let stdout = format!("[{}{}]", many, r#"{"title":"last"}"#);
        assert_eq!(parse(&stdout, &[]).len(), PLUGIN_UI_MAX_ROWS);
        let long = "x".repeat(500);
        let rows = parse(&format!(r#"[{{"title":"{long}"}}]"#), &[]);
        assert_eq!(rows[0].title.chars().count(), PLUGIN_UI_TITLE_MAX_CHARS);
    }

    #[test]
    fn workspace_tab_and_agent_events_touch_plugin_ui() {
        use crate::api::schema::EventKind as E;
        assert!(event_touches_plugin_ui(&E::TabFocused));
        assert!(event_touches_plugin_ui(&E::PaneAgentStatusChanged));
        assert!(event_touches_plugin_ui(&E::WorkspaceFocused));
        assert!(!event_touches_plugin_ui(&E::PaneUpdated));
    }
}
