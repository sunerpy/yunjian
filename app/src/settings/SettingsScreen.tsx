/**
 * 设置界面。
 *
 * # 五块内容，一条主线
 *
 * AI 服务商与密钥、语料库、语音模型、赏析缓存。主线是**如实呈现**：
 * 每一块都在说「此刻实际是什么状态」，而不是「设计上应该是什么状态」。
 * 最尖锐的一处是密钥存储位置——见 `storageIndicator.ts`。
 *
 * # 为什么本 todo 不把设置挂进 `App.tsx`
 *
 * `App.tsx` 是外壳共用文件，而 todo 63（背诵界面）此刻也在往里加屏。两个 todo 同时改同一个
 * 两态 `view` 联合会产生一次必然的冲突，而那次冲突解决起来只有一种正确答案（两屏都加）。
 * 所以本 todo 交付一个自包含、已测的 `SettingsScreen`，路由接线留给 IPC 与导航那一步统一做。
 * 这是刻意的取舍，不是遗漏——记在这里而不是留给读者猜。
 */

import type { SettingsPorts } from "../data/settingsPorts";
import CachePanel from "./CachePanel";
import CorpusPanel from "./CorpusPanel";
import KeyStoragePanel from "./KeyStoragePanel";
import ModelPanel from "./ModelPanel";
import "./settings.css";

export interface SettingsScreenProps {
  ports: SettingsPorts;
  /**
   * 当前提示词模板版本，交给缓存面板做「只清理本模板」。
   *
   * 默认值与 `AiConfig::default()` 的 `prompt_template_version`
   * （`crates/yunjian-core/src/config.rs:157-167`）一致，不另取一个数。
   */
  templateVersion?: string;
}

export default function SettingsScreen({ ports, templateVersion = "v1" }: SettingsScreenProps) {
  return (
    <div className="settings-screen" data-testid="settings-screen">
      <h1 className="settings-screen__title">设置</h1>
      <KeyStoragePanel keyStorePort={ports.keyStore} aiSettingsPort={ports.aiSettings} />
      <CorpusPanel port={ports.corpus} />
      <ModelPanel port={ports.models} />
      <CachePanel port={ports.cache} templateVersion={templateVersion} />
    </div>
  );
}
