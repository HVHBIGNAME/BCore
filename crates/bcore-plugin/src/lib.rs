//! Native plugin API and lifecycle/event manager for BCore.
mod context;
mod dynamic;
mod events;
mod manager;
mod plugin;
pub use context::{Command, CommandCallback, LogLevel, PluginContext};
pub use dynamic::DynamicPlugin;
pub use events::{EventHandler, ServerEvent};
pub use manager::PluginManager;
pub use plugin::{Plugin, PluginMetadata};

#[cfg(test)]
mod tests {
    use super::*;
    use bcore_core::{Identifier, Position};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    struct TestPlugin {
        enabled: Arc<AtomicBool>,
        heard: Arc<AtomicBool>,
    }
    impl Plugin for TestPlugin {
        fn id(&self) -> Identifier {
            Identifier::new("test", "plugin")
        }
        fn name(&self) -> &str {
            "Test"
        }
        fn version(&self) -> &str {
            "1.0"
        }
        fn authors(&self) -> Vec<String> {
            vec!["A".into()]
        }
        fn on_enable(&mut self, c: &mut PluginContext) {
            self.enabled.store(true, Ordering::SeqCst);
            let h = self.heard.clone();
            c.subscribe(move |e| {
                if matches!(e, ServerEvent::PlayerJoin { .. }) {
                    h.store(true, Ordering::SeqCst);
                }
            });
        }
    }
    #[test]
    fn registration_enable_and_dispatch() {
        let en = Arc::new(AtomicBool::new(false));
        let h = Arc::new(AtomicBool::new(false));
        let m = PluginManager::new();
        m.register(Box::new(TestPlugin {
            enabled: en.clone(),
            heard: h.clone(),
        }));
        m.enable_all();
        assert!(en.load(Ordering::SeqCst));
        m.fire_event(ServerEvent::PlayerJoin {
            player: "p".into(),
            position: Position::default(),
        });
        assert!(h.load(Ordering::SeqCst));
    }
    #[test]
    fn metadata_is_correct() {
        let p = TestPlugin {
            enabled: Arc::new(AtomicBool::new(false)),
            heard: Arc::new(AtomicBool::new(false)),
        };
        let md = PluginMetadata::from_plugin(&p);
        assert_eq!(md.id.to_string(), "test:plugin");
        assert_eq!(
            md.to_json(),
            r#"{"id":"test:plugin","name":"Test","version":"1.0","authors":["A"]}"#
        );
    }
    #[test]
    fn empty_dispatch_is_safe() {
        PluginManager::new().fire_event(ServerEvent::PlayerChat {
            player: "p".into(),
            message: "x".into(),
        });
    }
}
