/**
 * 非 Tauri 宿主下的设置端口替身，以及 Tauri 宿主下的 IPC 形状。
 *
 * # 样例数据在这里同样要自报身份
 *
 * `data/samplePorts.ts` 立下的规矩在设置界面上更要紧：一个开发者在 `vite dev` 里看到
 * 「系统钥匙串（持久）」，很容易以为产品在这台机器上真的做到了持久。
 * 所以样例的存储报告**刻意取本机真实会发生的那一档**——Linux 无头环境下降级到 keyutils，
 * 报告是 `(keyutils, login_session, os_encrypted)`，界面据此显示「重启后失效」。
 * 这也顺带让样例模式本身成为一次可见的诚实链演示。
 *
 * 样例里的密钥是**写进一个只在内存里的 Map**，且那个 Map 永不向界面暴露：
 * 替身也遵守「读回密钥的方法不存在」这条契约，否则替身会变成绕过契约的后门。
 */

import { invoke } from "@tauri-apps/api/core";
import type {
  AiSettings,
  CacheStatus,
  CorpusStatus,
  KeyStatus,
  ModelStatus,
  PurgeScope,
  StorageReport,
} from "../contracts/settings";
import { PROVIDER_NONE } from "../contracts/settings";
import type {
  AiSettingsPort,
  CachePort,
  CorpusPort,
  KeyStorePort,
  ModelPort,
  SettingsPorts,
} from "./settingsPorts";

/**
 * todo 64 要注册的命令名。
 *
 * 与 `data/tauriPorts.ts` 的 `IPC_COMMANDS` 同一条理由：命令名写错是**静默失败**，
 * 所以它必须是一个能被 grep 出来核对的清单。名字取 snake_case，与既有五个一致。
 */
export const SETTINGS_IPC_COMMANDS = {
  keyStatus: "key_status",
  setKey: "set_api_key",
  deleteKey: "delete_api_key",
  readAiSettings: "read_ai_settings",
  writeAiSettings: "write_ai_settings",
  corpusStatus: "corpus_status",
  fetchCorpus: "fetch_corpus",
  listModels: "list_models",
  cacheStatus: "cache_status",
  purgeCache: "purge_cache",
} as const;

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Tauri 宿主下的设置端口；不在宿主里时返回 `null`。 */
export function createTauriSettingsPorts(): SettingsPorts | null {
  if (!inTauri()) {
    return null;
  }

  const keyStore: KeyStorePort = {
    keyStatus: (provider: string) =>
      invoke<KeyStatus>(SETTINGS_IPC_COMMANDS.keyStatus, { provider }),
    // 密钥经参数进去，报告出来。**没有反方向的命令**，这一点由 `settingsPorts.ts` 的
    // 接口保证，这里只是照着实现。
    setKey: (provider: string, secret: string) =>
      invoke<StorageReport>(SETTINGS_IPC_COMMANDS.setKey, { provider, secret }),
    deleteKey: (provider: string) =>
      invoke<StorageReport>(SETTINGS_IPC_COMMANDS.deleteKey, { provider }),
  };

  const aiSettings: AiSettingsPort = {
    readAiSettings: () => invoke<AiSettings>(SETTINGS_IPC_COMMANDS.readAiSettings),
    writeAiSettings: (settings: AiSettings) =>
      invoke<void>(SETTINGS_IPC_COMMANDS.writeAiSettings, { settings }),
  };

  const corpus: CorpusPort = {
    corpusStatus: () => invoke<CorpusStatus>(SETTINGS_IPC_COMMANDS.corpusStatus),
    fetchCorpus: () => invoke<CorpusStatus>(SETTINGS_IPC_COMMANDS.fetchCorpus),
  };

  const models: ModelPort = {
    listModels: () => invoke<ModelStatus[]>(SETTINGS_IPC_COMMANDS.listModels),
  };

  const cache: CachePort = {
    cacheStatus: () => invoke<CacheStatus>(SETTINGS_IPC_COMMANDS.cacheStatus),
    purgeCache: (scope: PurgeScope) => invoke<number>(SETTINGS_IPC_COMMANDS.purgeCache, { scope }),
  };

  return { keyStore, aiSettings, corpus, models, cache };
}

/**
 * 样例存储报告：本机（无头 Linux）真实会落到的那一档。
 *
 * `location` 取 `Backend::Keyutils` 在 Rust 侧的位置串形态（后端名而非路径），
 * 与 `StorageReport.location` 的约定一致——「可显示的非机密字符串：路径或后端名」。
 */
const SAMPLE_STORED_REPORT: StorageReport = {
  backend: "keyutils",
  persistence: "login_session",
  protection: "os_encrypted",
  location: "linux keyutils（样例）",
};

/** 尚未存储时的报告。`persistence` 一律 `none`，见 `contracts/settings.ts`。 */
const SAMPLE_ABSENT_REPORT: StorageReport = {
  backend: "absent",
  persistence: "none",
  protection: "os_encrypted",
  location: "linux keyutils（样例）",
};

const SAMPLE_CORPUS: CorpusStatus = {
  kind: "ready",
  meta: {
    schema_version: 1,
    corpus_version: "sample-0.0.0",
    built_at: "2026-01-01T00:00:00Z",
    poem_count: 4,
    index_detail_mode: "column",
    derived_indexes: "derived_on_first_launch",
    shipped_scope: "样例数据（非语料）",
  },
};

/**
 * 样例模型清单。
 *
 * 三行刻意覆盖三种本机处境（已就位 / 只有归档 / 未下载），
 * 否则「未下载」这一行的渲染在测试与 dev 截图里都看不见。
 * 名称、许可与体积照 `models.toml` 里真实存在的条目写，不编造模型。
 */
const SAMPLE_MODELS: ModelStatus[] = [
  {
    name: "sherpa-onnx-whisper-tiny",
    kind: "asr",
    role: "production",
    license: "MIT",
    size_bytes: 116_204_861,
    unpacked: true,
    archived: true,
    refused: null,
    attribution: "openai-whisper.LICENSE",
  },
  {
    name: "sherpa-onnx-whisper-base",
    kind: "asr",
    role: "production",
    license: "MIT",
    size_bytes: 207_557_382,
    unpacked: false,
    archived: true,
    refused: null,
    attribution: "openai-whisper.LICENSE",
  },
  {
    name: "kokoro-multi-lang-v1_0",
    kind: "tts",
    role: "production",
    license: "Apache-2.0",
    size_bytes: 342_137_856,
    unpacked: false,
    archived: false,
    refused: null,
    attribution: "kokoro.LICENSE",
  },
];

/** 样例设置端口。所有方法都是 `async`，与 Tauri 那侧的形状一致。 */
export function createSampleSettingsPorts(): SettingsPorts {
  // 只写不读的密钥柜。**刻意不导出**，也刻意不提供读取入口：
  // 替身若能读回密钥，「读回的方法不存在」这条契约就只在生产代码里成立。
  const vault = new Map<string, string>();
  let aiSettings: AiSettings = {
    provider: PROVIDER_NONE,
    model: null,
    endpoint: null,
    temperature: 0.0,
    prompt_template_version: "v1",
  };
  let corpus: CorpusStatus = { kind: "absent" };
  let localRows = 2;

  const keyStore: KeyStorePort = {
    keyStatus: (provider: string) =>
      Promise.resolve(
        vault.has(provider)
          ? { report: SAMPLE_STORED_REPORT, needs_reprompt: false }
          : { report: SAMPLE_ABSENT_REPORT, needs_reprompt: true },
      ),
    setKey: (provider: string, secret: string) => {
      vault.set(provider, secret);
      return Promise.resolve(SAMPLE_STORED_REPORT);
    },
    deleteKey: (provider: string) => {
      vault.delete(provider);
      return Promise.resolve(SAMPLE_ABSENT_REPORT);
    },
  };

  return {
    keyStore,
    aiSettings: {
      readAiSettings: () => Promise.resolve(aiSettings),
      writeAiSettings: (next: AiSettings) => {
        aiSettings = next;
        return Promise.resolve();
      },
    },
    corpus: {
      corpusStatus: () => Promise.resolve(corpus),
      fetchCorpus: () => {
        corpus = SAMPLE_CORPUS;
        return Promise.resolve(corpus);
      },
    },
    models: {
      listModels: () => Promise.resolve(SAMPLE_MODELS),
    },
    cache: {
      cacheStatus: (): Promise<CacheStatus> =>
        // `database_bytes` 刻意缺省：样例宿主拿不到磁盘体积，界面应显示「未知」
        // 而不是编一个数字。这一条只有在样例里真的不给它时才会被验证到。
        Promise.resolve({ counts: { shipped: 300, local: localRows } }),
      purgeCache: (scope: PurgeScope) => {
        const removed = scope.kind === "all" ? localRows : Math.min(localRows, 1);
        localRows -= removed;
        return Promise.resolve(removed);
      },
    },
  };
}
