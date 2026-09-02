use crate::{
    context::PluginContext,
    events::{EventDispatcher, ServerEvent},
    Plugin,
};
use std::sync::{Arc, Mutex};

pub struct PluginManager {
    inner: Arc<Mutex<Inner>>,
}
struct Inner {
    plugins: Vec<Box<dyn Plugin>>,
    dispatcher: Arc<Mutex<EventDispatcher>>,
    enabled: bool,
}
impl Clone for PluginManager {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}
impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}
impl PluginManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                plugins: Vec::new(),
                dispatcher: Arc::new(Mutex::new(EventDispatcher::default())),
                enabled: false,
            })),
        }
    }
    pub fn register(&self, plugin: Box<dyn Plugin>) {
        self.inner.lock().unwrap().plugins.push(plugin);
    }
    pub fn enable_all(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner.enabled {
            return;
        }
        let dispatcher = inner.dispatcher.clone();
        for plugin in &mut inner.plugins {
            let mut ctx = PluginContext::new(dispatcher.clone());
            plugin.on_enable(&mut ctx);
        }
        inner.enabled = true;
    }
    pub fn disable_all(&self) {
        let mut inner = self.inner.lock().unwrap();
        if !inner.enabled {
            return;
        }
        for p in &mut inner.plugins {
            p.on_disable();
        }
        inner.enabled = false;
    }
    pub fn fire_event(&self, event: ServerEvent) {
        let inner = self.inner.lock().unwrap();
        inner.dispatcher.lock().unwrap().fire(&event);
    }
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().plugins.len()
    }
}
