/**
 * 阅读页的注记层偏好。
 *
 * # 为什么是两个键而不是一个对象
 *
 * 拼音层与平仄层是**两个独立开关，互不暗开**。把它们塞进同一个 JSON 对象里，
 * 「写一个键顺手把另一个键带上」就成了一次普通的手误——而这类手误的表现是「我只开了
 * 拼音，平仄自己也亮了」，用户会以为界面在替他做决定。两个独立键让「暗开」这件事必须
 * 显式地多写一次 `setItem` 才会发生。
 *
 * # 为什么不走 IPC
 *
 * 这是纯显示偏好：读它不需要语料库，写它不该等一次往返。落在 `localStorage` 上还有一个
 * 直接的好处——**切换开关不产生任何后端调用**，而那正是本功能的一条验收。
 *
 * # 读不到就是关
 *
 * 无痕模式、隐私设置或宿主不给 `localStorage` 时一律回落到「关」，并且**不抛异常**。
 * 注记层是学习支架，拿不到偏好时少显示一层是安全的；因为存储不可用而让详情页整页崩掉
 * 不是。
 */

/**
 * 两个开关各自的存储键。
 *
 * 键名带 `yunjian.` 前缀是为了在共享同一个 origin 的开发页面里不与别人撞车；
 * 集中在这里而不是散在调用点，是因为键名写错**不会报错**，只会表现成「设置没被记住」。
 */
export const ANNOTATION_PREFERENCE_KEYS = {
  pinyin: "yunjian.poem.showPinyin",
  tones: "yunjian.poem.showTones",
} as const;

/** 开关名。 */
export type AnnotationLayer = keyof typeof ANNOTATION_PREFERENCE_KEYS;

function storage(): Storage | null {
  // `localStorage` 的存取在被禁用时是**抛异常**而不是返回 null，所以取用本身要包起来。
  try {
    if (typeof globalThis.localStorage === "undefined") {
      return null;
    }
    return globalThis.localStorage;
  } catch {
    return null;
  }
}

/** 读一个开关。读不到、值不认识、存储不可用都算关。 */
export function readAnnotationPreference(layer: AnnotationLayer): boolean {
  try {
    return storage()?.getItem(ANNOTATION_PREFERENCE_KEYS[layer]) === "true";
  } catch {
    return false;
  }
}

/** 写一个开关。只碰它自己那个键。 */
export function writeAnnotationPreference(layer: AnnotationLayer, enabled: boolean): void {
  try {
    storage()?.setItem(ANNOTATION_PREFERENCE_KEYS[layer], enabled ? "true" : "false");
  } catch {
    // 写不进去不影响本次会话的显示，也不该打断阅读。
  }
}
