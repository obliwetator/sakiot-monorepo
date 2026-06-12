//! Path -> component mapping ported verbatim from ops/lib/components.sh.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Component {
    Database,
    Bot,
    Web,
    Frontend,
}

impl Component {
    pub const ALL: [Component; 4] = [
        Component::Database,
        Component::Bot,
        Component::Web,
        Component::Frontend,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Component::Database => "database",
            Component::Bot => "bot",
            Component::Web => "web",
            Component::Frontend => "frontend",
        }
    }
}

impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub fn all_components() -> Vec<Component> {
    Component::ALL.to_vec()
}

/// Maps changed paths to the components that must redeploy. Unknown paths
/// select everything (safe fallback). `ops/` is the deploy framework,
/// installed out-of-band into /usr/local/lib/sakiot-deploy and
/// /etc/systemd/system, not consumed from the release tag, so tag-time
/// ops/ changes do not rebuild artifacts.
pub fn components_for_paths<S: AsRef<str>>(paths: &[S]) -> Vec<Component> {
    if paths.is_empty() {
        return all_components();
    }

    let mut selected = [false; 4];
    let mut select = |components: &[Component]| {
        for component in components {
            selected[*component as usize] = true;
        }
    };

    for path in paths {
        let path = path.as_ref();
        if path.ends_with(".md") || path.starts_with("docs/") || path.starts_with("ops/") {
            // deploy framework / documentation: no component
        } else if path.starts_with("FBI-agent/") {
            select(&[Component::Bot]);
        } else if path.starts_with("web_server/") {
            select(&[Component::Web]);
        } else if path.starts_with("sakiot_stage/") {
            select(&[Component::Frontend]);
        } else if path.starts_with("sakiot-paths/")
            || path.starts_with("sakiot-proto/")
            || path == "Cargo.toml"
            || path == "Cargo.lock"
            || path.starts_with(".sqlx/")
        {
            select(&[Component::Bot, Component::Web]);
        } else if path.starts_with("sakiot-db/migrations/") {
            select(&[Component::Database, Component::Bot, Component::Web]);
        } else if path.starts_with("sakiot-db/ops/")
            || path.starts_with(".github/")
            || is_compose_file(path)
            || path == ".env.example"
        {
            select(&[Component::Bot, Component::Web, Component::Frontend]);
        } else {
            return all_components();
        }
    }

    Component::ALL
        .into_iter()
        .filter(|component| selected[*component as usize])
        .collect()
}

/// Bash globs `compose*.yml|compose*.yaml` match top-level files only (the
/// case patterns never contain a slash separator for these entries).
fn is_compose_file(path: &str) -> bool {
    path.starts_with("compose")
        && !path.contains('/')
        && (path.ends_with(".yml") || path.ends_with(".yaml"))
}

pub fn component_selected(wanted: Component, components: &[Component]) -> bool {
    components.contains(&wanted)
}

#[cfg(test)]
mod tests {
    //! Ported from ops/tests/components_test.sh.

    use super::Component::{Bot, Database, Frontend, Web};
    use super::*;

    fn assert_components(paths: &[&str], expected: &[Component]) {
        assert_eq!(
            components_for_paths(paths),
            expected.to_vec(),
            "paths: {paths:?}"
        );
    }

    #[test]
    fn empty_paths_select_all() {
        assert_components(&[], &[Database, Bot, Web, Frontend]);
    }

    #[test]
    fn docs_and_ops_select_nothing() {
        assert_components(&["README.md", "docs/notes.txt", "ops/deploy"], &[]);
        assert_components(&["STAGING.md", "ops/systemctl-wrapper"], &[]);
    }

    #[test]
    fn service_paths_map_to_their_component() {
        assert_components(&["FBI-agent/src/main.rs"], &[Bot]);
        assert_components(&["web_server/src/main.rs"], &[Web]);
        assert_components(&["sakiot_stage/src/App.tsx"], &[Frontend]);
    }

    #[test]
    fn shared_rust_paths_map_to_bot_and_web() {
        for path in [
            "sakiot-paths/src/lib.rs",
            "sakiot-proto/proto/fbi_agent.proto",
            "Cargo.toml",
            "Cargo.lock",
            ".sqlx/query-abc.json",
        ] {
            assert_components(&[path], &[Bot, Web]);
        }
    }

    #[test]
    fn migrations_select_database_and_services() {
        assert_components(
            &["sakiot-db/migrations/0007_new_table.sql"],
            &[Database, Bot, Web],
        );
    }

    #[test]
    fn infra_paths_select_services_and_frontend() {
        for path in [
            "sakiot-db/ops/backup/pre-migrate-backup.sh",
            ".github/workflows/ci.yml",
            "compose.dev.yml",
            ".env.example",
        ] {
            assert_components(&[path], &[Bot, Web, Frontend]);
        }
    }

    #[test]
    fn unknown_paths_select_all() {
        assert_components(&["mystery.txt"], &[Database, Bot, Web, Frontend]);
        assert_components(
            &["README.md", "mystery.txt"],
            &[Database, Bot, Web, Frontend],
        );
    }

    #[test]
    fn combinations_accumulate_in_canonical_order() {
        assert_components(
            &["web_server/src/main.rs", "FBI-agent/src/main.rs"],
            &[Bot, Web],
        );
        assert_components(
            &["sakiot_stage/src/App.tsx", "sakiot-db/migrations/0001.sql"],
            &[Database, Bot, Web, Frontend],
        );
    }

    #[test]
    fn nested_compose_files_are_not_compose_globs() {
        // compose*.yml in bash case matches the whole string; a nested path
        // such as deploy/compose.yml falls through to the unknown branch.
        assert_components(&["deploy/compose.yml"], &[Database, Bot, Web, Frontend]);
    }
}
