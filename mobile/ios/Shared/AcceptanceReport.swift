import Foundation
import XCTest

/// 把实测值搬回宿主的唯一出口。iOS 侧与 Android 的 `AcceptanceReport` 同职责、同协议。
///
/// # 协议必须逐字一致
///
/// 宿主侧 `xtask/src/acceptance/mobile/full_criteria.rs` 的 `MeasurementSet::parse` 只认两种行：
///
/// ```text
/// YUNJIAN-FULL <assertion> <key>=<value>
/// YUNJIAN-FULL <assertion> <key>_unavailable=<reason>
/// ```
///
/// 其余一概忽略。所以前缀、断言 id 与键名都不能在 iOS 上另起一套——那会让同一条判据在
/// 一个平台上量到值、在另一个平台上永远量成「未回传」。
///
/// # 两条通道，因为 XCUITest 与被测应用不在同一个进程
///
/// Android 的 instrumentation 跑在应用进程里，可以写应用私有文件；XCUITest 是**另一个进程**，
/// 拿不到被测应用的容器。所以这里走：
///
/// 1. `print` 到测试进程的 stdout —— Device Farm 的 run 日志会收；
/// 2. 测试进程沙箱内的文件 + `XCTAttachment` —— 随 result bundle 落盘，可离线取。
///
/// 两条都写。回收哪一条由设备侧脚本决定，与 Android 的三通道同一手法。
///
/// # 为什么这里不判阈值
///
/// 这个类**不认识** PASS 与 FAIL。判定在宿主侧完成，因为判词、阈值与报告 schema 都在那里。
/// 让被测物自己判等于把门禁搬进被测物内部——那种门禁在被测物坏掉时会一起坏掉。
///
/// # 尚未由 Xcode 编译验证
///
/// 本文件没有经过 Swift 编译器与真机运行（本机无 macOS）。见 `mobile/ios/README.md`。
enum AcceptanceReport {
    private static let prefix = "YUNJIAN-FULL"
    private static let fileName = "yunjian-acceptance.log"
    private static let lock = NSLock()

    /// 落盘位置。测试进程的临时目录在 result bundle 之外，所以同时做附件。
    static var logURL: URL {
        URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(fileName)
    }

    static func measure(_ assertion: String, _ key: String, _ value: Any?) {
        emit("\(prefix) \(assertion) \(key)=\(render(value))")
    }

    /// 这一项在本次运行中**没有测到**，并给出原因。
    ///
    /// 与 `measure(..., "")` 的区别是关键的：空串会被宿主侧读成「测到了一个空值」，进而记
    /// FAIL——把一次未到达说成产品失败。Android 的 spike 与 full 都记过这条。
    static func unavailable(_ assertion: String, _ key: String, reason: String) {
        emit("\(prefix) \(assertion) \(key)_unavailable=\(reason.replacingOccurrences(of: " ", with: "_"))")
    }

    static func note(_ message: String) {
        emit("\(prefix) note \(message.replacingOccurrences(of: "\n", with: " "))")
    }

    /// 把已积累的行作为附件挂到测试上。每条断言结束时调用一次即可（附件按测试归属）。
    static func attach(to testCase: XCTestCase) {
        guard let data = try? Data(contentsOf: logURL) else { return }
        let attachment = XCTAttachment(data: data, uniformTypeIdentifier: "public.plain-text")
        attachment.name = fileName
        attachment.lifetime = .keepAlways
        testCase.add(attachment)
    }

    private static func render(_ value: Any?) -> String {
        switch value {
        case .none:
            return "null"
        case .some(let wrapped as Bool):
            return wrapped ? "true" : "false"
        case .some(let wrapped as Int):
            return String(wrapped)
        case .some(let wrapped as Double):
            return String(wrapped)
        case .some(let wrapped):
            return String(describing: wrapped).replacingOccurrences(of: "\n", with: "/")
        }
    }

    /// 每行**追加**落盘。
    ///
    /// 与 Android 同一理由：原先每次都把内存里的全部行整份写下去，那在单进程里等价，但被测
    /// 进程被杀后 runner 会在新进程里继续跑，而新进程的内存是空的——于是第一次写就把前面
    /// 已经量到的值截掉。Android 第一轮真机实测正是如此：前六条断言的测量值全部消失，
    /// 读起来像「那几条没跑」。
    private static func emit(_ line: String) {
        print(line)
        lock.lock()
        defer { lock.unlock() }
        let payload = Data((line + "\n").utf8)
        if let handle = try? FileHandle(forWritingTo: logURL) {
            defer { try? handle.close() }
            _ = try? handle.seekToEnd()
            try? handle.write(contentsOf: payload)
        } else {
            try? payload.write(to: logURL)
        }
    }
}
