//! Android JNI 入口。**只做参数搬运**，逻辑全在 [`crate::corpus`] 里，这样宿主上的单元
//! 测试能覆盖到真正会出错的部分，而不是只能覆盖一个转发。
//!
//! 命名对应 Kotlin 侧的
//! `top.onethinker.yunjian.spike.SpikeCorpusBridge.measureCorpus(...)`。
//! 第二个参数用 [`JObject`] 而不是 `JClass`：Kotlin `object` 的成员函数编译成实例方法，
//! 而 `companion object` 加 `@JvmStatic` 编译成静态方法，两者传进来的分别是 `jobject`
//! 与 `jclass`。`JObject` 两种都接得住，于是 Kotlin 侧改写声明形态时这里不必跟着改。

use std::path::PathBuf;
use std::time::Duration;

use jni::JNIEnv;
use jni::objects::{JObject, JString};
use jni::sys::jstring;

use crate::corpus;

/// 走生产路径物化语料，返回一段 JSON 字符串。
///
/// 任何失败都以 JSON 形式返回，**不抛 Java 异常**：判据要的是「测到了什么」，
/// 而一个异常在 instrumentation 输出里只会变成一句堆栈，缺失项也就无从标注。
///
/// # Safety
///
/// 由 JVM 通过 `System.loadLibrary` 调用，参数由 JNI 规范保证有效。
// JNI 的查找方式是按名字取符号，因此导出名不能被 mangle。工作区把 `unsafe_code` 设成
// warn 并要求按点豁免，这里就是那个点：没有第二种办法让 JVM 找到这个函数。
#[allow(unsafe_code, reason = "JNI 按符号名查找，导出名不能 mangle")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_top_onethinker_yunjian_spike_SpikeCorpusBridge_measureCorpus<'local>(
    mut env: JNIEnv<'local>,
    _receiver: JObject<'local>,
    manifest_url: JString<'local>,
    data_root: JString<'local>,
    budget_seconds: jni::sys::jlong,
) -> jstring {
    let manifest = read_string(&mut env, &manifest_url).unwrap_or_default();
    let root = match read_string(&mut env, &data_root) {
        Some(value) if !value.trim().is_empty() => PathBuf::from(value),
        _ => {
            return json_out(
                &mut env,
                "{\"crashed\":true,\"failure\":\"未传入应用私有存储路径\"}",
            );
        }
    };
    let budget = if budget_seconds > 0 {
        Duration::from_secs(budget_seconds.unsigned_abs())
    } else {
        corpus::DEFAULT_BUDGET
    };

    let measured = corpus::measure(&manifest, &root, budget);
    let json = measured.to_json();
    json_out(&mut env, &json)
}

fn read_string(env: &mut JNIEnv<'_>, value: &JString<'_>) -> Option<String> {
    if value.is_null() {
        return None;
    }
    env.get_string(value).ok().map(Into::into)
}

/// 造一个 Java 字符串。造不出来时只能交回空指针，Kotlin 侧会把它当成缺失项上报——
/// 这仍然比返回一段假 JSON 好。
fn json_out(env: &mut JNIEnv<'_>, json: &str) -> jstring {
    env.new_string(json)
        .map_or(std::ptr::null_mut(), |value| value.into_raw())
}
