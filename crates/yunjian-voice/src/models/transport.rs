//! [`Transport`] 与 [`Unpacker`] 的生产实现。**只有本文件需要 HTTP 客户端与解压库。**
//!
//! 它被 `download` 特性挡着，而 `voice` 隐含 `download`——原生推理拿不到权重就没有意义，
//! 与 `voice` 隐含 `capture` 是同一条推理。
//!
//! 这一层刻意做得薄：进度、上限、摘要校验、原子落地全在 [`super`] 的判定层，那里由
//! `make ci` 的默认构建覆盖。这里只做两件不可能在没有依赖时完成的事——把 HTTPS 的字节
//! 读出来，和把 bzip2 压缩的 tar 解开。

use std::io::{Read as _, Write};
use std::path::{Component, Path};
use std::time::Duration;

use super::{Transport, Unpacker};

/// 连接与解析各自的超时。**刻意不设全局超时**：600 MB 的下载在慢网上可以正常跑很久，
/// 一个全局上限会把「慢」误判成「挂了」。真正需要上限的是握手阶段。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// 单次读取块大小。
const CHUNK_BYTES: usize = 1 << 16;

/// 基于 `ureq` 的 HTTPS 传输。
#[derive(Debug, Clone)]
pub struct HttpTransport {
    agent: ureq::Agent,
}

impl Default for HttpTransport {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_resolve(Some(CONNECT_TIMEOUT))
            .user_agent(concat!("yunjian/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Transport for HttpTransport {
    fn fetch(
        &self,
        url: &str,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<u64, String> {
        let mut response = self
            .agent
            .get(url)
            .call()
            .map_err(|error| error.to_string())?;

        let total = response.body().content_length().unwrap_or(0);
        // `as_reader()` 不设上限，这是刻意的：`ureq` 只给 `read_to_vec` 之类加 10 MiB
        // 上限，而我们的归档远超它。字节数的闸门在判定层的 `CappedWriter`，那里按清单
        // 记录的确切字节数卡，比任何固定上限都准。
        let mut reader = response.body_mut().as_reader();

        let mut buffer = vec![0_u8; CHUNK_BYTES];
        let mut done = 0_u64;
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            sink.write_all(&buffer[..read])
                .map_err(|error| error.to_string())?;
            done = done.saturating_add(read as u64);
            progress(done, total);
        }
        sink.flush().map_err(|error| error.to_string())?;
        Ok(done)
    }
}

/// 解 `.tar.bz2`。上游全部发布包都是这个形态。
#[derive(Debug, Clone, Copy, Default)]
pub struct Bz2TarUnpacker;

impl Unpacker for Bz2TarUnpacker {
    fn unpack(&self, archive: &Path, into: &Path) -> Result<(), String> {
        let file = std::fs::File::open(archive).map_err(|error| error.to_string())?;
        let reader = std::io::BufReader::with_capacity(CHUNK_BYTES, file);
        let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(reader));

        let entries = tar.entries().map_err(|error| error.to_string())?;
        for entry in entries {
            let mut entry = entry.map_err(|error| error.to_string())?;
            let path = entry
                .path()
                .map_err(|error| error.to_string())?
                .into_owned();
            reject_escaping_path(&path)?;
            // `unpack_in` 自己也拒绝逃出目标目录，返回 `false` 表示跳过了该条目。
            // 两道判断都留着：上面那条给出具体路径的诊断，这条是不依赖我们判断正确性的
            // 兜底。**这是解压下载内容时最容易被省掉的一步**，省掉它一个恶意归档就能
            // 往用户家目录任意位置写文件。
            let unpacked = entry.unpack_in(into).map_err(|error| error.to_string())?;
            if !unpacked {
                return Err(format!(
                    "归档条目 {} 被判定为不安全路径，已中止解包",
                    path.display()
                ));
            }
        }
        Ok(())
    }
}

fn reject_escaping_path(path: &Path) -> Result<(), String> {
    for component in path.components() {
        match component {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "归档条目 {} 含试图逃出目标目录的路径成分，已中止解包",
                    path.display()
                ));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Bz2TarUnpacker, reject_escaping_path};
    use crate::models::Unpacker as _;
    use std::path::{Path, PathBuf};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yunjian-transport-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("临时目录可创建");
        dir
    }

    #[test]
    fn escaping_paths_are_rejected_and_ordinary_ones_are_not() {
        for bad in ["../evil", "a/../../evil", "/etc/passwd"] {
            assert!(
                reject_escaping_path(Path::new(bad)).is_err(),
                "{bad} 必须被拒"
            );
        }
        for good in ["model/encoder.onnx", "./tokens.txt", "a/b/c"] {
            assert!(
                reject_escaping_path(Path::new(good)).is_ok(),
                "{good} 不该被拒"
            );
        }
    }

    /// 真的建一个 `.tar.bz2` 再解开，证明这一层不是只在类型上成立。
    #[test]
    fn a_real_bz2_tar_round_trips_through_the_unpacker() {
        let work = temp_dir("roundtrip");
        let payload = work.join("tokens.txt");
        std::fs::write(&payload, b"0 <blk>\n1 a\n").expect("写测试载荷");

        let archive = work.join("pack.tar.bz2");
        {
            let file = std::fs::File::create(&archive).expect("建归档");
            let encoder = bzip2::write::BzEncoder::new(file, bzip2::Compression::fast());
            let mut builder = tar::Builder::new(encoder);
            builder
                .append_path_with_name(&payload, "pack/tokens.txt")
                .expect("写归档条目");
            builder
                .into_inner()
                .expect("收尾 tar")
                .finish()
                .expect("收尾 bz2");
        }

        let into = work.join("out");
        std::fs::create_dir_all(&into).expect("建输出目录");
        Bz2TarUnpacker
            .unpack(&archive, &into)
            .expect("解包应当成功");

        let landed = into.join("pack").join("tokens.txt");
        assert_eq!(
            std::fs::read(&landed).expect("读解出的文件"),
            b"0 <blk>\n1 a\n",
            "解出的字节必须与打包前一致"
        );

        let _ = std::fs::remove_dir_all(&work);
    }
}
