use jni::objects::{JObject, JString, JValue};
use jni::{InitArgsBuilder, JNIEnv, JNIVersion, JavaVM};
use std::env;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum JavaBridgeError {
    JvmNotFound { searched: Vec<PathBuf> },
    InvalidPath(PathBuf),
    Jni(jni::errors::Error),
    Initialization(String),
}

impl fmt::Display for JavaBridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::JvmNotFound { searched } => {
                write!(f, "JVM library not found; searched: {searched:?}")
            }
            Self::InvalidPath(path) => write!(f, "path does not exist: {}", path.display()),
            Self::Jni(error) => write!(f, "JNI error: {error}"),
            Self::Initialization(error) => write!(f, "JVM initialization failed: {error}"),
        }
    }
}
impl std::error::Error for JavaBridgeError {}
impl From<jni::errors::Error> for JavaBridgeError {
    fn from(error: jni::errors::Error) -> Self {
        Self::Jni(error)
    }
}

/// Enabled plugin instance; retaining the loader keeps its classes alive.
#[allow(dead_code)]
pub struct JavaPluginHandle<'local> {
    loader: JObject<'local>,
    instance: JObject<'local>,
}

/// Embedded JVM entry point for legacy Java plugins.
pub struct JavaBridge {
    jvm: JavaVM,
    jar_path: PathBuf,
}

impl JavaBridge {
    /// Locate jvm.dll/libjvm.so using BCORE_JVM_LIB, JAVA_HOME and JDK layouts.
    pub fn find_jvm_library() -> Result<PathBuf, JavaBridgeError> {
        let mut searched = Vec::new();
        if let Ok(value) = env::var("BCORE_JVM_LIB") {
            let path = PathBuf::from(value);
            searched.push(path.clone());
            if path.is_file() {
                return Ok(path);
            }
        }
        if let Ok(home) = env::var("JAVA_HOME") {
            let candidates = Self::jvm_candidates(Path::new(&home));
            searched.extend(candidates.iter().cloned());
            if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
                return Ok(path);
            }
        }
        Err(JavaBridgeError::JvmNotFound { searched })
    }

    fn jvm_candidates(java_home: &Path) -> Vec<PathBuf> {
        #[cfg(windows)]
        {
            vec![
                java_home.join("bin/server/jvm.dll"),
                java_home.join("jre/bin/server/jvm.dll"),
            ]
        }
        #[cfg(not(windows))]
        {
            vec![
                java_home.join("lib/server/libjvm.so"),
                java_home.join("jre/lib/amd64/server/libjvm.so"),
            ]
        }
    }

    /// Create a JVM with the plugin jar and future API stubs on classpath.
    pub fn new(jar_path: impl AsRef<Path>) -> Result<Self, JavaBridgeError> {
        let jar_path = jar_path.as_ref().to_path_buf();
        if !jar_path.is_file() {
            return Err(JavaBridgeError::InvalidPath(jar_path));
        }
        Self::find_jvm_library()?;
        let classpath = format!("{}{}", jar_path.display(), classpath_suffix());
        let classpath_option = format!("-Djava.class.path={classpath}");
        let args = InitArgsBuilder::new()
            .version(JNIVersion::V8)
            .option(&classpath_option)
            .build()
            .map_err(|error| JavaBridgeError::Initialization(error.to_string()))?;
        let jvm = JavaVM::new(args)
            .map_err(|error| JavaBridgeError::Initialization(error.to_string()))?;
        Ok(Self { jvm, jar_path })
    }

    pub fn jar_path(&self) -> &Path {
        &self.jar_path
    }

    /// Load a class through URLClassLoader and invoke its public `onEnable()`.
    pub fn load_plugin<'local>(
        &self,
        env: &mut JNIEnv<'local>,
        plugin_class: &str,
    ) -> Result<JavaPluginHandle<'local>, JavaBridgeError> {
        let file_name = env.new_string(self.jar_path.to_string_lossy().as_ref())?;
        let file_arg = JObject::from(file_name);
        let file_class = env.find_class("java/io/File")?;
        let file = env.new_object(
            file_class,
            "(Ljava/lang/String;)V",
            &[JValue::Object(&file_arg)],
        )?;
        let uri = env
            .call_method(&file, "toURI", "()Ljava/net/URI;", &[])?
            .l()?;
        let url = env
            .call_method(&uri, "toURL", "()Ljava/net/URL;", &[])?
            .l()?;
        let urls = env.new_object_array(1, "java/net/URL", JObject::null())?;
        env.set_object_array_element(&urls, 0, &url)?;
        let loader_class = env.find_class("java/net/URLClassLoader")?;
        let loader = env
            .call_static_method(
                loader_class,
                "newInstance",
                "([Ljava/net/URL;)Ljava/net/URLClassLoader;",
                &[JValue::Object(&urls)],
            )?
            .l()?;
        let class_name = env.new_string(plugin_class)?;
        let class_arg = JObject::from(class_name);
        let clazz = env
            .call_method(
                &loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(&class_arg)],
            )?
            .l()?;
        let instance = env
            .call_method(&clazz, "newInstance", "()Ljava/lang/Object;", &[])?
            .l()?;
        env.call_method(&instance, "onEnable", "()V", &[])?;
        Ok(JavaPluginHandle { loader, instance })
    }

    /// Proof-of-life JNI call. Every thread using JNI must be attached first.
    pub fn java_version(&self) -> Result<String, JavaBridgeError> {
        let mut env = self.jvm.attach_current_thread()?;
        let key = env.new_string("java.version")?;
        let key_obj = JObject::from(key);
        let system = env.find_class("java/lang/System")?;
        let value = env
            .call_static_method(
                system,
                "getProperty",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::Object(&key_obj)],
            )?
            .l()?;
        Ok(env
            .get_string(&JString::from(value))?
            .to_string_lossy()
            .into_owned())
    }
}

// Architecture seam: `org/bukkit/*` stub classes declare `native` methods.
// Native Rust registers them with `JNIEnv::register_native_methods`; handlers
// validate arguments, call bcore-core, and convert Results to Java values or
// exceptions. This exports getServer/getLogger/registerCommand and events from
// native Rust without translating plugin bytecode. No panic may cross JNI.
fn classpath_suffix() -> &'static str {
    if cfg!(windows) {
        ";stubs.jar"
    } else {
        ":stubs.jar"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a local JDK and a plugin jar"]
    fn jvm_bridge_end_to_end() {
        // Note: a JVM can only be created once per process, so this single test
        // covers both proof-of-life JNI and the M1 plugin-load path with one JVM.
        let jar = std::env::var("BCORE_PLUGIN_TEST_JAR").expect("set BCORE_PLUGIN_TEST_JAR");
        let bridge = JavaBridge::new(jar).expect("JVM should load");

        // Proof of life: JNI round-trip to the embedded JVM.
        assert!(!bridge
            .java_version()
            .expect("JNI call should work")
            .is_empty());

        // M1: load a plugin class through URLClassLoader and invoke onEnable.
        let mut env = bridge.jvm.attach_current_thread().expect("attach thread");
        let _handle = bridge
            .load_plugin(&mut env, "bcore.example.ExamplePlugin")
            .expect("load plugin class and invoke onEnable");
    }
}
