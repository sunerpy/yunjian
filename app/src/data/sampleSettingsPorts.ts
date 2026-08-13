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
 * 样例宿主能演示的四档存储，键名即 `?sample-key-tier=` 的取值。
 *
 * # 为什么需要这个开关
 *
 * 默认那一档是**本机（无头 Linux）真实会落到的** keyutils，这让样例模式本身成为一次
 * 诚实链演示。但另外三档在样例宿主里就永远到不了——尤其是明文那一档，
 * 而它恰恰是唯一会显示告警的一档。「告警写了但没人看过它长什么样」是本项目
 * 在深色令牌上已经栽过的那类问题（见 `__tests__/theme.test.ts` 的来历）。
 *
 * # 为什么用查询参数，而不是加一个界面控件
 *
 * 加控件会在**产品**里留下一个只对调试有意义的开关，将来 todo 64 还得把它摘掉。
 * 查询参数只被 `createSampleSettingsPorts` 读到，而那个函数只在非 Tauri 宿主里被调用
 * ——真实桌面端走 `createTauriSettingsPorts`，永远读不到它。样例宿主本来就有一条常驻
 * 横幅声明「这里的一切都不是语料」，多一个调试入口不改变它的性质。
 *
 * `location` 的形态与 `StorageReport.location` 的约定一致：可显示的非机密字符串
 * （路径或后端名）。
 */
const SAMPLE_TIERS: Record<string, StorageReport> = {
  keyutils: {
    backend: "keyutils",
    persistence: "login_session",
    protection: "os_encrypted",
    location: "linux keyutils（样例）",
  },
  keychain: {
    backend: "secret_service",
    persistence: "persistent",
    protection: "os_encrypted",
    location: "GNOME Keyring（样例）",
  },
  session: {
    backend: "session_memory",
    persistence: "process_only",
    protection: "process_memory",
    location: "本进程内存（样例）",
  },
  plaintext: {
    backend: "plaintext_file",
    persistence: "persistent",
    protection: "plaintext",
    location: "~/.config/yunjian/keys.toml（样例）",
  },
};

/** 默认档：本机无头 Linux 真实会落到的那一档。 */
const DEFAULT_TIER = "keyutils";

/**
 * 显式指定档位时预置密钥所用的服务商。
 *
 * 取 `PROVIDER_IDS` 里的第一个真实服务商，**不能取 `PROVIDER_NONE`**：
 * 「在没有服务商的情况下存了一枚密钥」本身是个矛盾状态，而密钥的 account 名就是
 * 服务商标识。
 */
const PRESET_PROVIDER = "deepseek";

function requestedTier(): { report: StorageReport; preset: boolean } {
  if (typeof window === "undefined") {
    return { report: SAMPLE_TIERS[DEFAULT_TIER] as StorageReport, preset: false };
  }
  const asked = new URLSearchParams(window.location.search).get("sample-key-tier");
  const found = asked === null ? undefined : SAMPLE_TIERS[asked];
  // 显式指定了档位就顺带预置一枚密钥，否则要点一次保存才看得到指示串。
  // 不指定时刻意保持空柜：首次启动没有密钥，那是真实的首启态。
  return {
    report: found ?? (SAMPLE_TIERS[DEFAULT_TIER] as StorageReport),
    preset: found !== undefined,
  };
}

/** 尚未存储时的报告。`persistence` 一律 `none`，见 `contracts/settings.ts`。 */
function absentReport(stored: StorageReport): StorageReport {
  return {
    backend: "absent",
    persistence: "none",
    // `backend: absent` 时这两项描述的是「若存在会在哪、受什么保护」，
    // 所以沿用当前档位的值而不是写死。
    protection: stored.protection,
    location: stored.location,
  };
}

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
  const { report: stored, preset } = requestedTier();
  const absent = absentReport(stored);
  // 只写不读的密钥柜。**刻意不导出**，也刻意不提供读取入口：
  // 替身若能读回密钥，「读回的方法不存在」这条契约就只在生产代码里成立。
  const vault = new Map<string, string>();
  let aiSettings: AiSettings = {
    // 默认「不配置服务商」，与 `AiConfig::default()` 一致。
    provider: PROVIDER_NONE,
    model: null,
    endpoint: null,
    temperature: 0.0,
    prompt_template_version: "v1",
  };
  if (preset) {
    // **两者必须一起设**。第一版只 `vault.set(PRESET_PROVIDER, …)` 而没动
    // `aiSettings.provider`，于是面板首屏查的是 `keyStatus("none")`——那把柜子里根本没有
    // 「none」这个 account，指示串永远落在「尚未存储」，`?sample-key-tier=` 看起来完全没生效。
    //
    // 那个缺陷没被 9 条档位断言拦住，因为它们硬写了 `keyStatus(PRESET_PROVIDER)`，
    // **绕过了「面板实际会查哪个 provider」这一段**。密钥的 account 名就是服务商标识
    // （`ProviderKind::as_str()` 的注释：「同时用作钥匙串里的 account 名」），
    // 所以「有一枚已存的密钥」与「已选定服务商」在语义上本来就是同一件事，
    // 分开设置只会造出一个自相矛盾的状态。
    aiSettings = { ...aiSettings, provider: PRESET_PROVIDER };
    // 预置值本身也读不出来：柜子只被 `has` 查询，从不被 `get`。
    vault.set(PRESET_PROVIDER, "sample-preset");
  }
  let corpus: CorpusStatus = { kind: "absent" };
  let localRows = 2;

  const keyStore: KeyStorePort = {
    keyStatus: (provider: string) =>
      Promise.resolve(
        vault.has(provider)
          ? { report: stored, needs_reprompt: false }
          : { report: absent, needs_reprompt: true },
      ),
    setKey: (provider: string, secret: string) => {
      vault.set(provider, secret);
      return Promise.resolve(stored);
    },
    deleteKey: (provider: string) => {
      vault.delete(provider);
      return Promise.resolve(absent);
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
