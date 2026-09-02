use crate::events::EventDispatcher;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

pub type CommandCallback = Arc<dyn Fn(&[&str]) + Send + Sync + 'static>;
#[derive(Clone)]
pub struct Command {
    pub name: String,
    pub callback: CommandCallback,
}

#[derive(Clone)]
pub struct PluginContext {
    dispatcher: Arc<Mutex<EventDispatcher>>,
    commands: Arc<Mutex<Vec<Command>>>,
    logger: Arc<dyn Fn(LogLevel, &str) + Send + Sync + 'static>,
}
impl PluginContext {
    pub(crate) fn new(dispatcher: Arc<Mutex<EventDispatcher>>) -> Self {
        Self {
            dispatcher,
            commands: Arc::new(Mutex::new(Vec::new())),
            logger: Arc::new(|level, msg| eprintln!("[{level:?}] {msg}")),
        }
    }
    pub fn log(&self, level: LogLevel, message: &str) {
        (self.logger)(level, message);
    }
    pub fn register_command<F>(&self, name: impl Into<String>, callback: F)
    where
        F: Fn(&[&str]) + Send + Sync + 'static,
    {
        self.commands.lock().unwrap().push(Command {
            name: name.into(),
            callback: Arc::new(callback),
        });
    }
    pub fn commands(&self) -> Vec<Command> {
        self.commands.lock().unwrap().clone()
    }
    pub fn subscribe<F>(&self, handler: F)
    where
        F: Fn(&crate::ServerEvent) + Send + Sync + 'static,
    {
        self.dispatcher.lock().unwrap().subscribe(Arc::new(handler));
    }
}
