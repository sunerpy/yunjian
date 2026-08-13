[简体中文](../PLATFORM-REQUIREMENTS.zh.md) · English

# Platform requirements

Yunjian's voice features (read-aloud and spoken recitation) have OS version floors. **A system below a
floor gets no voice, but it gets the complete typed-practice product, not a broken application.**
Dictionary search, the three typed recitation modes, FSRS review and the MCP server are all
independent of voice and none of them is constrained by this table.

This document is pinned by `cargo test -p yunjian-voice --test platform_config`: every version number
below must match `FLOORS` in `crates/yunjian-voice/src/platform.rs` verbatim, and changing one place
while missing another fails the test.

## Contents

- [The five per-platform floors](#the-five-per-platform-floors)
- [Where each floor comes from](#where-each-floor-comes-from)
- [The microphone permission chain](#the-microphone-permission-chain)
- [Behaviour below the floor](#behaviour-below-the-floor)
- [Verification status](#verification-status)

## The five per-platform floors

| Platform    | Minimum version | Floor set by                    |
| ----------- | --------------- | ------------------------------- |
| **Linux**   | glibc 2.31      | The release build host          |
| **Windows** | Windows 10 1809 | WebView2 evergreen distribution |
| **macOS**   | 14.2            | The `cpal` CoreAudio backend    |
| **Android** | 8.0 (API 26)    | The `cpal` AAudio backend       |
| **iOS**     | 14.0            | The Tauri v2 deployment target  |

## Where each floor comes from

### macOS 14.2 — confirmed true

The research claim that `cpal` requires macOS 14.2 **holds**, but the cause differs from "upstream
documents say so"; it was established by reading the code:

`cpal`'s macOS backend has a loopback path (recording system output), and
`src/host/coreaudio/macos/loopback.rs` unconditionally references
`AudioHardwareCreateProcessTap`, `AudioHardwareDestroyProcessTap` and `CATapDescription` — three APIs
**introduced in macOS 14.2**. They are bound through `objc2-core-audio`, and that crate uses a plain
`#[link(name = "CoreAudio", kind = "framework")] extern "C" {}` with **no weak linking**. So even if
loopback is never enabled, the symbols are in the binary and loading fails on systems earlier than
14.2.

Both candidate `cpal` versions (0.17.3 inside `rodio 0.22.2`, and the 0.18.1 the plan once pinned
directly) **have this path**, so changing versions does not avoid it.

`bundle.macOS.minimumSystemVersion` is therefore written explicitly as `14.2`, overriding Tauri's
default of `10.13`: leaving the default would not fail the build, it would just let users on
10.13–14.1 install an application that crashes on launch.

### Android API 26 — higher than the 24 in Tauri's documentation

`cpal`'s Android backend binds AAudio through the `ndk` crate's **`api-level-26`** feature
(`[target.'cfg(target_os = "android")'.dependencies.ndk] features = ["audio", "api-level-26"]`,
byte-identical in 0.17.3 and 0.18.1), while Tauri's documentation states an Android minimum of 7.0 /
**API 24**.

**The conflict is resolved as the plan required: `minSdk` is raised to 26 and recorded here as a
product requirement.** It lands in three places, with a test asserting all three agree:

- `minSdk = 26` in `crates/yunjian-voice/mobile/android/build.gradle.kts`
- `bundle.android.minSdkVersion = 26` in `crates/yunjian-voice/mobile/tauri.audio.conf.json`
- the linker hard-coded as `aarch64-linux-android26-clang` in CI

Staying at 24 would not fail to compile; it would **crash at runtime on API 24/25 devices**, because
those levels have no AAudio symbols. That is exactly why it has to be a hard constraint in the
configuration layer rather than a note in a document.

### iOS 14.0 — the floor comes from Tauri, not the audio stack

The `AVAudioSession` capture path itself exists on much earlier iOS. 14.0 comes from Tauri v2's
default `IPHONEOS_DEPLOYMENT_TARGET`, and `bundle.iOS.minimumSystemVersion` is written out explicitly so
that an upstream default change cannot move it silently.

The plugin branches internally on iOS 17: `AVAudioApplication.requestRecordPermission` exists only from
17.0, and earlier systems use the deprecated
`AVAudioSession.sharedInstance().recordPermission`. Both paths are present, so 14.0–16.x are
unaffected.

### Windows 10 1809 — the floor comes from WebView2

WASAPI capture itself has existed since Vista. 1809 comes from the WebView2 evergreen runtime that
Tauri v2 depends on, which no longer supports earlier Windows 10 releases. The desktop shell cannot
run at all without WebView2, so the audio floor is not the deciding factor here.

### Linux glibc 2.31

The ALSA backend needs `libasound2` (`libasound2-dev` at build time). 2.31 is Ubuntu 20.04's glibc and
the oldest build host in the release matrix; a dynamically-linked artifact cannot run on a glibc older
than its build host.

## The microphone permission chain

**Capturing in Rust bypasses the WebView, but it does not bypass the operating system.** Every platform
requires an authorization, and each places that authorization at a completely different layer:

| Platform | Authorization point                                                             | Who can initiate it                                |
| -------- | ------------------------------------------------------------------------------- | -------------------------------------------------- |
| Linux    | No system-level microphone gate                                                 | The process itself                                 |
| Windows  | Settings → Privacy → Microphone, triggered by the first WASAPI capture          | The process itself                                 |
| macOS    | A TCC dialog, triggered when a **signed** process first touches an input device | The process itself (but entitlements are required) |
| Android  | A runtime permission dialog                                                     | **Only the Android framework**                     |
| iOS      | Record authorization plus `AVAudioSession` activation                           | Requires a native plugin; `cpal` does not do it    |

### Android needs two permissions, not one

```
android.permission.RECORD_AUDIO
android.permission.MODIFY_AUDIO_SETTINGS
```

- `RECORD_AUDIO` is dangerous-level, must be requested at runtime, and can be refused.
- `MODIFY_AUDIO_SETTINGS` is normal-level and **granted at install time by declaration alone, with no
  runtime request**; but when the declaration is missing, `getUserMedia` on the WebView side fails
  outright (`tauri#10846`), and native capture on some devices cannot obtain a usable input route
  either, because audio mode and route switching fall under it.

The Kotlin plugin reports the two under separate aliases and additionally reports a
"permanently denied" verdict derived from the negation of
`shouldShowRequestPermissionRationale` — once the user has ticked "don't ask again",
`requestPermissions` **returns a denial callback immediately without showing a dialog**, and at that
point the UI must direct the user to system settings rather than to another button press.

### macOS needs all three, and none is optional

1. `NSMicrophoneUsageDescription` in `Info.plist` — without it, the process is **terminated outright**
   by the system the instant it touches an input device; it does not receive an error.
2. A **separate** `Entitlements.plist` containing two keys:
   ```
   com.apple.security.device.microphone
   com.apple.security.device.audio-input
   ```
3. `bundle.macOS.entitlements` pointing at that file.

**This configuration only fails when missing in a signed and notarized build** (`tauri#8314`):
`tauri dev` and local unsigned builds are entirely green. That is exactly why it has to be covered by a
static assertion in the configuration layer together with `codesign -d --entitlements -` in CI, and
cannot rely on observation during development.

### iOS must activate `AVAudioSession` first

Reading `cpal 0.18.1`'s `src/host/coreaudio/ios/mod.rs` confirms it: it obtains
`AVAudioSession.sharedInstance()` and sets `setPreferredIOBufferDuration`, but **never calls
`setCategory` or `setActive`**. And it decides whether a device has input from
`inputNumberOfChannels()`, which is **0** while the session is inactive and the category is still the
default `.soloAmbient` — so `cpal` reports "no input device", presenting as a path that neither errors
nor records anything.

The Swift plugin therefore activates the session as `.playAndRecord` plus `.measurement`:
`.playAndRecord` because the read-aloud demonstration and the spoken recitation alternate within the
same screen and `.record` would cut the output route; `.measurement` because it disables the system's
automatic gain control and noise suppression, both of which harm recognition.

## Behaviour below the floor

**Every failure path returns to typed practice with an explanatory message.** This is guaranteed by the
type system rather than by convention: `VoiceError::degrade_reason()` is an exhaustive match, so adding
an error variant without giving it a degradation reason does not compile.

| Situation                       | Degradation reason       | What the user sees                                         |
| ------------------------------- | ------------------------ | ---------------------------------------------------------- |
| Voice not compiled in           | `FeatureDisabled`        | Switched to typed practice; the scoring kernel is the same |
| System version too old          | `SystemTooOld`           | The required version number                                |
| Authorization denied            | `PermissionDenied`       | That platform's settings path                              |
| Disabled by a management policy | `PermissionRestricted`   | An explanation that an administrator must allow it         |
| Not yet authorized              | `PermissionUndetermined` | An explanation that pressing start will prompt             |
| No input device                 | `NoInputDevice`          | A prompt to attach a microphone or change device           |
| Capture failed or truncated     | `CaptureFailed`          | A note that another program may hold it exclusively        |

**Truncated capture is treated as a failure, not as success returning half a recording.** Silently
accepting truncation would hand the caller half a second of audio labelled "one second", the recitation
scoring would treat it as an unfinished attempt, and a device fault would become a **wrong score**
rather than a visible failure. One measured instance of `alsa::poll() spuriously returned` delivered
only 484 ms for a one-second request.

The criterion is a **completion ratio of 0.95**, not "not one sample may be missing": the resampler
gives up a few dozen samples at span boundaries (measured at 15936/16000 in CI, i.e. 996 ms), which is
a 4 ms rounding rather than a fault. There is an order of magnitude of clearance between the two
measured numbers (0.996 vs 0.484), so this gate is not arbitrary.

Every message must answer two things: why recording is unavailable, and how to recover. A test asserts
that all thirty-five messages (seven reasons × five platforms) mention typed practice, so that the
product never leaves a user stranded with just "the microphone is unavailable".

## Verification status

Stated honestly: this spike's development machine is Linux, with no macOS/Android/iOS host, no physical
device and no signing identity. The table has two columns: **Local** is the measured conclusion on the
development machine, and **CI** is the real conclusion `audio-permissions.yml` obtained where a
suitable host exists. Any item without a pass in either column carries its blocking cause and what it
would need.

| Item                                                    | Local                                | CI               | Evidence / blocking cause                                                                                                                                                                                                                                                                                                |
| ------------------------------------------------------- | ------------------------------------ | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Linux: capture 1 s of 16 kHz mono with non-zero RMS     | **Pass**                             | **Pass**         | Locally a PulseAudio null sink plus `module-sine`, RMS 0.353550; CI takes the same path, with setup first verifying `peak=16384` via `parec`                                                                                                                                                                             |
| macOS: capture 1 s of 16 kHz mono with non-zero RMS     | Not executed                         | **Pass**         | No macOS host. CI installs BlackHole 2ch on `macos-14`, restarts `coreaudiod` and captures RMS 0.353544; **the device is natively 2-channel, so the downmix path was genuinely exercised** rather than the format happening to match                                                                                     |
| macOS: entitlements reach the signed artifact           | Not executed                         | **Pass**         | No signing identity. CI uses an ad-hoc signature plus `codesign -d --entitlements -` to read both keys back, with a reverse check: signing without `--entitlements` genuinely fails to yield `audio-input`                                                                                                               |
| macOS: runtime behaviour of a notarized build           | Not executed                         | **Not executed** | Needs a paid Apple Developer account and notarization credentials. The failure described in `tauri#8314` surfaces only at this layer; belongs to the release process                                                                                                                                                     |
| macOS: TCC authorization dialog                         | Not executed                         | **Not executed** | The runner has no GUI session, and a command-line process does not trigger the TCC dialog. Needs a signed artifact plus a real machine                                                                                                                                                                                   |
| Windows: real capture                                   | Not executed                         | **Partial pass** | The runner has neither a microphone nor a non-interactive virtual input driver (enumeration returned 0 devices). What is verified is the degradation path: it reports `NoInputDevice` with an explanation including the settings path, without panicking or hanging. Real capture needs a Windows host with a sound card |
| Android: build the capture path                         | Not executed                         | **Pass**         | No NDK locally. CI uses r26d, and the linker resolves to `aarch64-linux-android26-clang` (pinned by an assertion that it is not 24)                                                                                                                                                                                      |
| Android: runtime permission dialog                      | Not executed                         | **Not executed** | Needs a device or emulator. CI only builds; on-device acceptance is deferred                                                                                                                                                                                                                                             |
| iOS: build the capture path                             | Not executed                         | **Pass**         | No Xcode SDK locally. `cargo build` succeeds on `macos-14` in CI — this workflow does not enable `voice`, so it is unaffected by `sherpa-rs`'s cdylib limitation                                                                                                                                                         |
| iOS: on-device capture and authorization                | Not executed                         | **Not executed** | Needs macOS, Xcode and a device with a configured signing identity                                                                                                                                                                                                                                                       |
| The macOS 14.2 floor                                    | **Confirmed (by reading code)**      | —                | `loopback.rs` references 14.2 APIs unconditionally and `objc2-core-audio` does no weak linking. Runtime failure on 14.1 and earlier is **not** measured — no host of that version exists here                                                                                                                            |
| The Android API 26 floor                                | **Confirmed (by reading manifests)** | —                | The `ndk` crate's `api-level-26` feature, byte-identical in cpal 0.17.3 and 0.18.1                                                                                                                                                                                                                                       |
| `AVAudioSession` activation is required on iOS          | **Confirmed (by reading code)**      | —                | `setActive` / `setCategory` occur zero times in cpal's iOS backend                                                                                                                                                                                                                                                       |
| Degradation to typed practice when permission is denied | **Pass**                             | **Pass**         | All five platforms × seven reasons produce an explanation including the settings path; verified in reverse (making it not degrade produced 5 harness findings and matching unit-test failures)                                                                                                                           |

## Related documents

- [Voice](VOICE.md) — models and licences, the 破读 lexicon, the v1 feedback contract (no pronunciation-standard assessment)
- [Voice build](../VOICE-BUILD.zh.md) — native dependency builds across five platforms, linking, GPL-3.0 impact
- [Architecture](ARCHITECTURE.md) — layering and the mobile escape hatch
- [Third-party licences](../../LICENSES.md) — per-asset licence and attribution
