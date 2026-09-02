use crate::{Plugin, PluginMetadata};
use libloading::{Library, Symbol};
use std::{
    error::Error,
    ffi::{c_void, CStr},
    path::Path,
};

pub struct DynamicPlugin {
    _library: Library,
    plugin: Box<dyn Plugin>,
}
impl DynamicPlugin {
    pub unsafe fn load(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let library = Library::new(path.as_ref())?;
        let create: Symbol<unsafe extern "C" fn() -> *mut c_void> =
            library.get(b"bcore_plugin_create")?;
        let raw = create();
        if raw.is_null() {
            return Err("bcore_plugin_create returned null".into());
        }
        let plugin = Box::from_raw(raw as *mut Box<dyn Plugin>);
        Ok(Self {
            _library: library,
            plugin: *plugin,
        })
    }
    pub unsafe fn metadata(path: impl AsRef<Path>) -> Result<PluginMetadata, Box<dyn Error>> {
        let library = Library::new(path.as_ref())?;
        let symbol: Symbol<unsafe extern "C" fn() -> *const std::os::raw::c_char> =
            library.get(b"bcore_plugin_metadata")?;
        let ptr = symbol();
        if ptr.is_null() {
            return Err("null metadata".into());
        }
        let json = CStr::from_ptr(ptr).to_string_lossy();
        parse_metadata(&json).ok_or_else(|| "invalid plugin metadata JSON".into())
    }
}
impl Plugin for DynamicPlugin {
    fn id(&self) -> bcore_core::Identifier {
        self.plugin.id()
    }
    fn name(&self) -> &str {
        self.plugin.name()
    }
    fn version(&self) -> &str {
        self.plugin.version()
    }
    fn authors(&self) -> Vec<String> {
        self.plugin.authors()
    }
    fn on_enable(&mut self, c: &mut crate::PluginContext) {
        self.plugin.on_enable(c)
    }
    fn on_disable(&mut self) {
        self.plugin.on_disable()
    }
}
fn parse_metadata(s: &str) -> Option<PluginMetadata> {
    fn val(s: &str, key: &str) -> Option<String> {
        let p = format!("\"{key}\":\"");
        let a = s.find(&p)? + p.len();
        let b = s[a..].find('"')?;
        Some(s[a..a + b].replace("\\\"", "\"").replace("\\\\", "\\"))
    }
    let id = bcore_core::Identifier::parse(&val(s, "id")?)?;
    let authors = val(s, "authors")
        .map(|x| x.split('|').map(str::to_owned).collect())
        .unwrap_or_default();
    Some(PluginMetadata {
        id,
        name: val(s, "name")?,
        version: val(s, "version")?,
        authors,
    })
}
