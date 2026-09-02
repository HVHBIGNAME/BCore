use crate::context::PluginContext;
use bcore_core::Identifier;

pub trait Plugin: Send {
    fn id(&self) -> Identifier;
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn authors(&self) -> Vec<String>;
    fn on_enable(&mut self, context: &mut PluginContext);
    fn on_disable(&mut self) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMetadata {
    pub id: Identifier,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
}
impl PluginMetadata {
    pub fn from_plugin(plugin: &dyn Plugin) -> Self {
        Self {
            id: plugin.id(),
            name: plugin.name().into(),
            version: plugin.version().into(),
            authors: plugin.authors(),
        }
    }
    pub fn to_json(&self) -> String {
        fn q(s: &str) -> String {
            format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
        }
        format!(
            "{{\"id\":{},\"name\":{},\"version\":{},\"authors\":[{}]}}",
            q(&self.id.to_string()),
            q(&self.name),
            q(&self.version),
            self.authors
                .iter()
                .map(|a| q(a))
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}
