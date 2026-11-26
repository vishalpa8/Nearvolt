//! Opens the files a user picked, by asking the app's Kotlin side over JNI. Each
//! file is opened only when the sender reaches it.

use std::fs::File;
use std::io;
use std::os::fd::FromRawFd;
use std::sync::OnceLock;

use jni::objects::{GlobalRef, JClass, JObject, JValue};
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
        let fd = match result {
            Ok(fd) => fd,
            Err(e) => {
                let _ = env.exception_clear();
                return Err(io::Error::other(e));
            }
        };
        if fd < 0 {
            return Err(io::Error::new(io::ErrorKind::NotFound, "the file is no longer available"));
        }
        // Kotlin detaches the descriptor, so this File is its only owner.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
