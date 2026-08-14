/**
 * 设置界面的行为断言：密钥不回显、明文警告、语料事实、模型逐项许可。
 *
 * # 「密钥永不回显」这一条是怎么测的
 *
 * 分三层查，因为前两层单独都能被绕过：
 *
 * 1. 输入框的 `value` 为空——最直接，但只看这一条的话，密钥出现在别处（另一个只读框、
 *    一个 `title` 属性、一段状态文本）就漏了；
 * 2. **整棵 DOM 的 `outerHTML` 里搜不到那个串**——`outerHTML` 同时覆盖文本节点与属性值，
 *    所以 `title="sk-..."` 或 `data-key="sk-..."` 也拦得住；
 * 3. **每个 `input` / `textarea` 的 `.value` 都搜不到**——受控 input 的 value 不落进
 *    `outerHTML`，只有逐个读 `.value` 才看得见。第 2 条单独存在时这里是个真实缺口。
 *
 * 三层齐备，才配得上「没有任何测试能从 DOM 读到那个值」这句话。
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type {
  AiSettings,
  CacheStatus,
  CorpusStatus,
  KeyStatus,
  ModelStatus,
  StorageReport,
} from "../../contracts/settings";
import type { SettingsPorts } from "../../data/settingsPorts";
import SettingsScreen from "../SettingsScreen";

const SECRET = "sk-TESTKEY123-must-never-be-echoed";

function report(overrides: Partial<StorageReport> = {}): StorageReport {
  return {
    backend: "keyutils",
    persistence: "login_session",
    protection: "os_encrypted",
    location: "linux keyutils",
    ...overrides,
  };
}

function models(): ModelStatus[] {
  return [
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
    {
      name: "vits-zh-hf-fanchen-C",
      kind: "tts",
      role: "smoke",
      license: "unknown",
      size_bytes: 1_024,
      unpacked: false,
      archived: false,
      refused: "许可未核实，不在 MIT / Apache-2.0 允许列表内",
      attribution: "（无）",
    },
  ];
}

const CORPUS_READY: CorpusStatus = {
  kind: "ready",
  meta: {
    schema_version: 3,
    corpus_version: "tang-song-2026.08",
    built_at: "2026-08-01T12:00:00Z",
    poem_count: 470_123,
    index_detail_mode: "column",
    derived_indexes: "derived_on_first_launch",
    shipped_scope: "唐宋",
  },
};

interface Overrides {
  storedReport?: StorageReport;
  absentReport?: StorageReport;
  corpus?: CorpusStatus;
  cache?: CacheStatus;
  aiSettings?: Partial<AiSettings>;
}

/** 端口替身。`setKey` 记录收到的密钥，供「写入方向仍然通」的断言使用。 */
function createPorts(overrides: Overrides = {}): {
  ports: SettingsPorts;
  received: string[];
  setKey: ReturnType<typeof vi.fn>;
} {
  const stored = overrides.storedReport ?? report();
  const absent =
    overrides.absentReport ??
    report({
      backend: "absent",
      persistence: "none",
      protection: "os_encrypted",
    });
  const received: string[] = [];
  let has = false;

  const setKey = vi.fn((_provider: string, secret: string) => {
    received.push(secret);
    has = true;
    return Promise.resolve(stored);
  });

  const ports: SettingsPorts = {
    keyStore: {
      keyStatus: (): Promise<KeyStatus> =>
        Promise.resolve(
          has
            ? { report: stored, needs_reprompt: false }
            : { report: absent, needs_reprompt: true },
        ),
      setKey,
      deleteKey: () => {
        has = false;
        return Promise.resolve(absent);
      },
    },
    aiSettings: {
      readAiSettings: () =>
        Promise.resolve({
          provider: "deepseek",
          model: null,
          endpoint: null,
          temperature: 0.0,
          prompt_template_version: "v1",
          ...overrides.aiSettings,
        }),
      writeAiSettings: () => Promise.resolve(),
    },
    corpus: {
      corpusStatus: () => Promise.resolve(overrides.corpus ?? CORPUS_READY),
      fetchCorpus: () => Promise.resolve(CORPUS_READY),
    },
    models: {
      listModels: () => Promise.resolve(models()),
    },
    cache: {
      cacheStatus: () => Promise.resolve(overrides.cache ?? { counts: { shipped: 312, local: 7 } }),
      purgeCache: () => Promise.resolve(7),
    },
  };

  return { ports, received, setKey };
}

/** 整棵 DOM 里是否出现过某个串，含属性值与受控 input 的 value。 */
function domContains(needle: string): boolean {
  if (document.body.outerHTML.includes(needle)) {
    return true;
  }
  const fields = document.body.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>(
    "input, textarea",
  );
  return [...fields].some((field) => field.value.includes(needle));
}

describe("标题", () => {
  it("独立渲染时带「设置」这个 h1", () => {
    // 装进弹窗时它被关掉（弹窗头部条已经写着「设置」），但独立形态默认必须带标题——
    // 否则这一屏没有任何东西说明自己是什么。默认值站在独立形态这一边，弹窗是特例。
    render(<SettingsScreen ports={createPorts().ports} />);
    const heading = screen.getByRole("heading", { level: 1 });
    expect(heading.textContent).toBe("设置");
  });

  it("`showTitle={false}` 时不渲染任何 h1", () => {
    render(<SettingsScreen ports={createPorts().ports} showTitle={false} />);
    expect(screen.queryByRole("heading", { level: 1 })).toBeNull();
    // 四块面板仍然都在：关掉的只是标题，不是内容。
    for (const label of ["AI 服务商与密钥", "语料库", "语音模型", "赏析缓存"]) {
      expect(screen.getByLabelText(label), `缺少「${label}」面板`).toBeTruthy();
    }
  });
});

describe("密钥输入", () => {
  it("是遮罩输入，且关掉了自动填充与拼写检查", async () => {
    render(<SettingsScreen ports={createPorts().ports} />);
    const input = (await screen.findByTestId("api-key-input")) as HTMLInputElement;
    expect(input.type).toBe("password");
    expect(input.autocomplete).toBe("off");
    expect(input.getAttribute("spellcheck")).toBe("false");
  });

  it("保存后输入框为空，且整棵 DOM 里读不到那个密钥", async () => {
    const { ports, received } = createPorts();
    render(<SettingsScreen ports={ports} />);
    const input = (await screen.findByTestId("api-key-input")) as HTMLInputElement;

    fireEvent.change(input, { target: { value: SECRET } });
    // 输入期间它当然在框里——那是用户正在打字。这一条只是确认测试真的输进去了。
    expect(input.value).toBe(SECRET);

    fireEvent.click(screen.getByTestId("save-key"));
    await waitFor(() => {
      expect(screen.getByTestId("key-notice").textContent).toContain("密钥已保存");
    });

    // 第 1 层：输入框为空。
    expect((screen.getByTestId("api-key-input") as HTMLInputElement).value).toBe("");
    // 第 2、3 层：文本节点、属性值、以及所有受控 input 的 value 都搜不到。
    expect(domContains(SECRET)).toBe(false);
    // 写入方向仍然通——否则上面三条可以靠「压根没保存」轻松满足。
    expect(received).toEqual([SECRET]);
  });

  it("保存成功后 placeholder 说明已保存，但不显示任何密钥内容", async () => {
    const { ports } = createPorts();
    render(<SettingsScreen ports={ports} />);
    const input = (await screen.findByTestId("api-key-input")) as HTMLInputElement;
    fireEvent.change(input, { target: { value: SECRET } });
    fireEvent.click(screen.getByTestId("save-key"));
    await waitFor(() => {
      expect((screen.getByTestId("api-key-input") as HTMLInputElement).placeholder).toContain(
        "已保存",
      );
    });
    expect((screen.getByTestId("api-key-input") as HTMLInputElement).placeholder).not.toContain(
      "sk-",
    );
    expect(domContains(SECRET)).toBe(false);
  });

  it("删除密钥后同样读不到，且提示需要重新输入", async () => {
    const { ports } = createPorts();
    render(<SettingsScreen ports={ports} />);
    const input = (await screen.findByTestId("api-key-input")) as HTMLInputElement;
    fireEvent.change(input, { target: { value: SECRET } });
    fireEvent.click(screen.getByTestId("save-key"));
    await waitFor(() => {
      expect(screen.getByTestId("key-notice")).toBeTruthy();
    });

    fireEvent.click(screen.getByTestId("delete-key"));
    await waitFor(() => {
      expect(screen.getByTestId("key-notice").textContent).toContain("密钥已删除");
    });
    expect(screen.getByTestId("key-storage-indicator").textContent).toContain("尚未存储");
    expect(screen.getByTestId("key-needs-reprompt")).toBeTruthy();
    expect(domContains(SECRET)).toBe(false);
  });

  it("空草稿时保存按钮不可用", async () => {
    render(<SettingsScreen ports={createPorts().ports} />);
    await waitFor(() => {
      expect((screen.getByTestId("save-key") as HTMLButtonElement).disabled).toBe(true);
    });
  });
});

describe("存储位置指示条", () => {
  it("keyutils 报告渲染「系统密钥环（重启后失效）」而不是钥匙串", async () => {
    const { ports } = createPorts();
    render(<SettingsScreen ports={ports} />);
    const input = (await screen.findByTestId("api-key-input")) as HTMLInputElement;
    fireEvent.change(input, { target: { value: SECRET } });
    fireEvent.click(screen.getByTestId("save-key"));

    await waitFor(() => {
      expect(screen.getByTestId("key-storage-indicator").textContent).toContain(
        "系统密钥环（重启后失效）",
      );
    });
    const indicator = screen.getByTestId("key-storage-indicator");
    expect(indicator.textContent).not.toContain("系统钥匙串");
    expect(indicator.textContent).not.toContain("持久");
    expect(screen.getByTestId("key-storage-detail").textContent).toContain("重启或注销后失效");
    // 非明文时不出警告条：警告要在明文那一档才出现，否则它会退化成常驻装饰。
    expect(screen.queryByTestId("plaintext-warning")).toBeNull();
  });

  it("会话内存报告渲染「仅本次会话」", async () => {
    const { ports } = createPorts({
      storedReport: report({
        backend: "session_memory",
        persistence: "process_only",
        protection: "process_memory",
        location: "本进程内存",
      }),
    });
    render(<SettingsScreen ports={ports} />);
    const input = (await screen.findByTestId("api-key-input")) as HTMLInputElement;
    fireEvent.change(input, { target: { value: SECRET } });
    fireEvent.click(screen.getByTestId("save-key"));
    await waitFor(() => {
      expect(screen.getByTestId("key-storage-indicator").textContent).toContain("仅本次会话");
    });
    expect(screen.getByTestId("key-storage-indicator").textContent).not.toContain("系统钥匙串");
  });

  it("Windows 凭据管理器报告则渲染「系统钥匙串（持久）」", async () => {
    const { ports } = createPorts({
      storedReport: report({
        backend: "windows_credential",
        persistence: "persistent",
        protection: "os_encrypted",
        location: "Windows 凭据管理器",
      }),
    });
    render(<SettingsScreen ports={ports} />);
    const input = (await screen.findByTestId("api-key-input")) as HTMLInputElement;
    fireEvent.change(input, { target: { value: SECRET } });
    fireEvent.click(screen.getByTestId("save-key"));
    await waitFor(() => {
      expect(screen.getByTestId("key-storage-indicator").textContent).toContain(
        "系统钥匙串（持久）",
      );
    });
  });
});

describe("明文配置文件", () => {
  it("渲染显式警告，且警告条带 role=alert 与醒目徽标", async () => {
    const { ports } = createPorts({
      storedReport: report({
        backend: "plaintext_file",
        persistence: "persistent",
        protection: "plaintext",
        location: "/home/u/.config/yunjian/keys.toml",
      }),
    });
    render(<SettingsScreen ports={ports} />);
    const input = (await screen.findByTestId("api-key-input")) as HTMLInputElement;
    fireEvent.change(input, { target: { value: SECRET } });
    fireEvent.click(screen.getByTestId("save-key"));

    await waitFor(() => {
      expect(screen.getByTestId("plaintext-warning")).toBeTruthy();
    });
    const warning = screen.getByTestId("plaintext-warning");
    expect(warning.getAttribute("role")).toBe("alert");
    expect(warning.textContent).toContain("未加密");
    expect(warning.textContent).toContain("/home/u/.config/yunjian/keys.toml");
    expect(screen.getByTestId("plaintext-warning-badge").textContent).toContain("明文存储警告");
    expect(screen.getByTestId("key-storage-indicator").textContent).toContain("明文配置文件");
    // 明文那一档持久性确实是 persistent，但**不许说成「系统钥匙串」**。
    expect(screen.getByTestId("key-storage-indicator").textContent).not.toContain("系统钥匙串");
    // 密钥确实在那儿，所以这一次才允许用陈述语气。
    expect(warning.getAttribute("data-mood")).toBe("actual");
    expect(warning.textContent).toContain("密钥以明文保存在");
  });

  it("告警带图标，不只靠颜色与文字承载语义", async () => {
    const { ports } = createPorts({
      absentReport: report({
        backend: "absent",
        persistence: "none",
        protection: "plaintext",
        location: "~/.config/yunjian/keys.toml",
      }),
    });
    render(<SettingsScreen ports={ports} />);
    await waitFor(() => {
      expect(screen.getByTestId("plaintext-warning-badge").textContent).toContain("⚠");
    });
    // 图标对读屏隐藏：紧邻的文字已经说了同一件事。
    const icon = screen
      .getByTestId("plaintext-warning-badge")
      .querySelector('[aria-hidden="true"]');
    expect(icon?.textContent).toBe("⚠");
  });

  it("**尚未存储时告警改用条件式措辞，不得断言密钥已明文保存**", async () => {
    // 这一条来自一次真实的假话。第一版只看 `protection`，于是无密钥时那一屏同时出现
    // 「尚未保存任何密钥」与「密钥**以明文保存在** …」——后者在当时是编造的。
    // 与把 keyutils 报成持久是同一类错误，只是方向相反（夸大风险而非隐瞒风险）。
    const { ports } = createPorts({
      absentReport: report({
        backend: "absent",
        persistence: "none",
        protection: "plaintext",
        location: "~/.config/yunjian/keys.toml",
      }),
    });
    render(<SettingsScreen ports={ports} />);

    await waitFor(() => {
      expect(screen.getByTestId("key-storage-indicator").textContent).toContain("尚未存储");
    });
    // 告警**仍然显示**：首启就告知代价，比存进去之后才警告有用得多。
    const warning = screen.getByTestId("plaintext-warning");
    expect(warning.getAttribute("data-mood")).toBe("prospective");
    expect(warning.textContent).toContain("若在此保存密钥");
    // 陈述语气一个字都不许出现。
    expect(warning.textContent).not.toContain("密钥以明文保存在");
    // 后果照旧说清楚——换语气不是弱化风险。
    expect(warning.textContent).toContain("未加密");
    expect(warning.textContent).toContain("一旦被备份、同步或打包带走");
  });

  it("整屏文案不自相矛盾：说了「尚未保存」就不许同时说「已保存」", async () => {
    // 单测各自断言一句话时，每一句单看都是对的——那个缺陷只有看整屏才暴露。
    // 所以这一条对着**整棵 DOM 的文本**断言，而不是某一个节点。
    const { ports } = createPorts({
      absentReport: report({
        backend: "absent",
        persistence: "none",
        protection: "plaintext",
        location: "~/.config/yunjian/keys.toml",
      }),
    });
    render(<SettingsScreen ports={ports} />);
    await waitFor(() => {
      expect(screen.getByTestId("plaintext-warning")).toBeTruthy();
    });
    const screenText = screen.getByTestId("settings-screen").textContent ?? "";
    expect(screenText).toContain("尚未保存任何密钥");
    expect(screenText).not.toContain("密钥以明文保存在");
  });
});

describe("语料状态面板", () => {
  it("显示来自 corpus_meta 的版本与记录数", async () => {
    render(<SettingsScreen ports={createPorts().ports} />);
    await waitFor(() => {
      expect(screen.getByTestId("corpus-version").textContent).toContain("tang-song-2026.08");
    });
    // 记录数字段名是 `poem_count`；猜成 `record_count` 这里会显示 undefined。
    expect(screen.getByTestId("corpus-poem-count").textContent).toContain("470,123");
    expect(screen.getByTestId("corpus-schema-version").textContent).toContain("3");
    expect(screen.getByTestId("corpus-index-mode").textContent).toContain("column");
    expect(screen.getByTestId("corpus-shipped-scope").textContent).toContain("唐宋");
  });

  it("没有语料库时是一句陈述加一个下载按钮，不是错误横幅", async () => {
    render(<SettingsScreen ports={createPorts({ corpus: { kind: "absent" } }).ports} />);
    await waitFor(() => {
      expect(screen.getByTestId("corpus-absent")).toBeTruthy();
    });
    expect(screen.queryByTestId("corpus-error")).toBeNull();
    expect(screen.getByTestId("fetch-corpus").textContent).toContain("下载语料库");
  });

  it("取用动作把状态换成已就绪", async () => {
    render(<SettingsScreen ports={createPorts({ corpus: { kind: "absent" } }).ports} />);
    await waitFor(() => {
      expect(screen.getByTestId("fetch-corpus")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("fetch-corpus"));
    await waitFor(() => {
      expect(screen.getByTestId("corpus-version").textContent).toContain("tang-song-2026.08");
    });
  });
});

describe("模型管理列表", () => {
  it("每一项都显示许可，一行不漏", async () => {
    render(<SettingsScreen ports={createPorts().ports} />);
    await waitFor(() => {
      expect(screen.getAllByTestId("model-row")).toHaveLength(3);
    });
    const rows = screen.getAllByTestId("model-row");
    const licenses = screen.getAllByTestId("model-license");
    // 行数与许可标签数必须相等：少一个就说明有一行没显示许可。
    expect(licenses).toHaveLength(rows.length);
    for (const license of licenses) {
      expect(license.textContent?.replace("许可 ", "")).toBeTruthy();
    }
    expect(licenses.map((node) => node.textContent)).toEqual([
      "许可 MIT",
      "许可 Apache-2.0",
      "许可 unknown",
    ]);
  });

  it("显示体积与本机状态三态", async () => {
    render(<SettingsScreen ports={createPorts().ports} />);
    await waitFor(() => {
      expect(screen.getAllByTestId("model-presence")).toHaveLength(3);
    });
    expect(screen.getAllByTestId("model-presence").map((node) => node.textContent)).toEqual([
      "已就位",
      "未下载",
      "未下载",
    ]);
    expect(screen.getAllByTestId("model-size").map((node) => node.textContent)).toEqual([
      "110.8 MiB",
      "326.3 MiB",
      "1.0 KiB",
    ]);
  });

  it("被许可门禁拒掉的模型仍然列出，并显示理由", async () => {
    render(<SettingsScreen ports={createPorts().ports} />);
    await waitFor(() => {
      expect(screen.getByTestId("model-refused")).toBeTruthy();
    });
    expect(screen.getByTestId("model-refused").textContent).toContain("许可未核实");
  });
});

describe("缓存管理", () => {
  it("随包与本机两层分开显示，体积拿不到时显示未知", async () => {
    render(<SettingsScreen ports={createPorts().ports} />);
    await waitFor(() => {
      expect(screen.getByTestId("cache-shipped").textContent).toContain("312");
    });
    expect(screen.getByTestId("cache-local").textContent).toContain("7");
    // 体积在 Rust 侧没有对应 API，替身刻意不给——界面必须说「未知」而不是编一个数。
    expect(screen.getByTestId("cache-bytes").textContent).toContain("未知");
  });

  it("给了体积就按 1024 进制显示", async () => {
    render(
      <SettingsScreen
        ports={
          createPorts({
            cache: {
              counts: { shipped: 1, local: 2 },
              database_bytes: 5_242_880,
            },
          }).ports
        }
      />,
    );
    await waitFor(() => {
      expect(screen.getByTestId("cache-bytes").textContent).toContain("5.0 MiB");
    });
  });

  it("清理后说明随包层未受影响", async () => {
    render(<SettingsScreen ports={createPorts().ports} />);
    await waitFor(() => {
      expect(screen.getByTestId("purge-all")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("purge-all"));
    await waitFor(() => {
      expect(screen.getByTestId("cache-notice").textContent).toContain("随包预生成的赏析未受影响");
    });
  });

  it("按模板清理的按钮带上当前模板版本", async () => {
    render(<SettingsScreen ports={createPorts().ports} templateVersion="1.0.0" />);
    await waitFor(() => {
      expect(screen.getByTestId("purge-template").textContent).toContain("1.0.0");
    });
  });
});
