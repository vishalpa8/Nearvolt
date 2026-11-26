//! Opens the files a user picked, by asking the app's Kotlin side over JNI. Each
//! file is opened only when the sender reaches it.

use std::fs::File;
use std::io;
use std::os::fd::FromRawFd;
use std::sync::OnceLock;

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};
use nearvolt_core::SourceOpener;

static BRIDGE: OnceLock<(JavaVM, GlobalRef)> = OnceLock::new();

/// Called once by `NativeBridge` in Kotlin at startup.
#[no_mangle]
pub extern "system" fn Java_app_nearvolt_NativeBridge_register(env: JNIEnv, _class: JClass, bridge: JObject) {
    if let (Ok(vm), Ok(bridge)) = (env.get_java_vm(), env.new_global_ref(bridge)) {
        let _ = BRIDGE.set((vm, bridge));
    }
}

/// Files picked in one selection, identified by the token Kotlin handed out.
pub struct PickedFiles {
    pub token: i64,
}

impl SourceOpener for PickedFiles {
    fn open(&self, index: u32) -> io::Result<File> {
        let (vm, bridge) =
            BRIDGE.get().ok_or_else(|| io::Error::other("the app has not registered its file access"))?;
        let mut env = vm.attach_current_thread_permanently().map_err(io::Error::other)?;
        let result = env
            .call_method(bridge, "openSource", "(JI)I", &[JValue::Long(self.token), JValue::Int(index as i32)])
            .and_then(|value| value.i());
        let fd = result.map_err(|e| io::Error::other(java_error(&mut env, e)))?;
        if fd < 0 {
            return Err(io::Error::new(io::ErrorKind::NotFound, "the file is no longer available"));
        }
        // Kotlin detaches the descriptor, so this File is its only owner.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

/// The pending Java exception as text, such as `java.lang.SecurityException: …`,
/// instead of jni's generic "Java exception was thrown". Clears it, so later
/// calls on this thread work.
fn java_error(env: &mut JNIEnv, error: jni::errors::Error) -> String {
    let throwable = env.exception_occurred().ok().filter(|t| !t.is_null());
    let _ = env.exception_clear();
    throwable
        .and_then(|t| env.call_method(&t, "toString", "()Ljava/lang/String;", &[]).ok())
        .and_then(|text| text.l().ok())
        .and_then(|text| env.get_string(&JString::from(text)).ok().map(String::from))
        .unwrap_or_else(|| error.to_string())
}
