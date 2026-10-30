// Unit test for src/rules/recorderLogic.ts: the set-recording options, the dB meter, the clock with his pauses and the
// system's, and which mic tapes the room.   node --experimental-strip-types --no-warnings tests/recorder.test.mjs
import * as R from "../src/rules/recorderLogic.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const near = (a, b, eps = 1e-6) => Math.abs(a - b) < eps;

// ---- the options (expo-audio 58 RecordingOptions; the literals match build/RecordingConstants.js HIGH_QUALITY)
const O = R.SET_RECORDING;
check(O.extension === ".m4a" && O.ios.outputFormat === "aac " && O.android.audioEncoder === "aac", "AAC in an .m4a: the pipeline's AUDIO_EXT");
check(O.sampleRate === 48000 && O.numberOfChannels === 2, "48 kHz, two channels");
check(O.bitRate >= 128000 && O.bitRate <= 192000, `bit rate ${O.bitRate} suits speech plus laughs (128-192 kbps)`);
check(O.isMeteringEnabled === true, "metering on (the level bars)");
check(O.directory === "document", "written to the document directory, not the purgeable cache");
check(O.ios.audioQuality === 127, "AudioQuality.MAX");
check(typeof R.KEEP_AWAKE_TAG === "string" && R.KEEP_AWAKE_TAG.length > 0, "a keep-awake tag to release on stop");

// ---- the meter
check(R.dbToLinear(0) === 1 && near(R.dbToLinear(-20), 0.1) && near(R.dbToLinear(-6.0206), 0.5, 1e-4), "dBFS -> amplitude: 0 dB = 1, -20 dB = 0.1, -6 dB = 0.5");
check(R.dbToLinear(-160) < 1e-7 && R.dbToLinear(6) === 1, "amplitude clamps to 0..1");
check(R.dbToLinear(undefined) === 0 && R.dbToLinear(NaN) === 0 && R.dbToLinear(-Infinity) === 0 && R.dbToLinear(null) === 0, "no reading -> 0");
check(R.meterLevel(0) === 1 && R.meterLevel(-30) === 0.5 && R.meterLevel(-60) === 0 && R.meterLevel(-160) === 0, "bar height over a -60 dBFS floor: 0 dB full, -30 dB half, floor and below empty");
check(R.meterLevel(-25, -50) === 0.5 && R.meterLevel(-10, 0) === 0, "a custom floor; a non-negative floor is refused");
check(R.meterLevel(undefined) === 0 && R.meterLevel(NaN) === 0 && R.meterLevel(5) === 1, "no reading -> 0; above 0 dB clamps to 1");

// ---- the clock
let c = R.clockStart(1000);
check(R.clockElapsed(c, 1000) === 0 && R.clockElapsed(c, 6000) === 5000, "runs from the start");
c = R.clockPause(c, 6000, "user");
check(R.clockElapsed(c, 9000) === 5000 && c.by === "user", "his pause stops the clock");
check(R.clockPause(c, 7000, "user") === c, "a second pause is a no-op (pausedAt stays)");
c = R.clockObserve(c, false, 8000);
check(c.by === "user" && R.clockElapsed(c, 9000) === 5000, "the system reporting 'not recording' during his pause changes nothing");
c = R.clockResume(c, 9000);
check(c.by === "none" && c.pausedTotal === 3000 && R.clockElapsed(c, 11000) === 7000, "resume: 3 s paused is taken off");
check(R.clockResume(c, 12000) === c, "resume when not paused is a no-op");
// a phone call: native stops recording without a Pause tap
c = R.clockObserve(c, true, 11000);
check(c.by === "none" && R.clockElapsed(c, 11000) === 7000, "recording as expected: the clock runs");
c = R.clockObserve(c, false, 12000);
check(c.by === "system" && R.clockElapsed(c, 20000) === 8000, "the call: a system gap opens, the clock holds at 8 s");
c = R.clockObserve(c, false, 15000);
check(c.pausedAt === 12000, "the gap keeps its start while the call goes on");
c = R.clockObserve(c, true, 20000);
check(c.by === "none" && c.pausedTotal === 11000 && R.clockElapsed(c, 25000) === 13000, "the recorder comes back: 8 s of call taken off, the clock runs again");
check(R.clockElapsed(R.clockStart(5000), 4000) === 0, "never negative (a clock skew)");
const u = R.clockPause(R.clockStart(0), 1000, "user");
check(R.clockObserve(u, true, 2000) === u, "native 'recording' during his pause does not resume his pause (he does)");

// ---- the input
const builtIn = { uid: "b", name: "iPhone Microphone", type: "MicrophoneBuiltIn" };
const wired = { uid: "w", name: "Headset", type: "MicrophoneWired" };
const usb = { uid: "u", name: "USB mic", type: "UsbAudio" };
const bt = { uid: "h", name: "AirPods", type: "BluetoothHFP" };
check(R.pickInput([builtIn, bt], bt)?.uid === "b", "a headset connected: the phone's own mic, not Bluetooth HFP");
check(R.pickInput([builtIn, bt], builtIn) === null, "already on the built-in mic: nothing to set");
check(R.pickInput([builtIn, wired], builtIn)?.uid === "w" && R.pickInput([bt, usb, builtIn], builtIn)?.uid === "u", "a wired or USB mic beats the phone's");
check(R.pickInput([bt], bt) === null && R.pickInput([bt], null)?.uid === "h", "Bluetooth is the only input: leave it");
check(R.pickInput([], null) === null, "no inputs: null");
check(R.pickInput([builtIn, { uid: "x", type: { rawValue: "UsbAudio" } }], builtIn)?.uid === "x", "the struct shape of the port type (getAvailableInputs) is read too");
check(R.pickInput([{ uid: "q", type: 42 }, builtIn], null)?.uid === "b", "an unknown type ranks below the built-in mic");

// the record control's state machine (2026-10-29): nothing records until start; stop from recording OR paused saves
check(R.nextRecordState("idle", "start") === "recording" && R.nextRecordState("recording", "start") === "recording", "start only from idle");
check(R.nextRecordState("recording", "pause") === "paused" && R.nextRecordState("idle", "pause") === "idle", "pause only while recording");
check(R.nextRecordState("paused", "resume") === "recording" && R.nextRecordState("recording", "resume") === "recording", "resume only from paused");
check(R.nextRecordState("recording", "stop") === "saving" && R.nextRecordState("paused", "stop") === "saving" && R.nextRecordState("idle", "stop") === "idle", "stop saves from recording or paused, never from idle");
check(R.nextRecordState("saving", "start") === "saving" && R.nextRecordState("saving", "stop") === "saving", "nothing moves out of saving but the upload");
check(R.statusWord("idle", false, false) === "Ready" && R.statusWord("idle", false, true) === "Clock only", "idle reads Ready, or Clock only after Next bit");
check(R.statusWord("recording", false, false) === "Recording" && R.statusWord("recording", true, false) === "Paused by a call", "a call reads as such");
check(R.statusWord("paused", false, false) === "Paused" && R.statusWord("saving", false, false) === "Saving…", "paused and saving");
// 2026-10-29 (build 12 "recording didn't work properly"): a save that fails leaves the native recorder stopped, so the
// control returns to idle (a fresh take), never to paused (a Resume that cannot resume); a save that lands leaves too
check(R.nextRecordState("saving", "failed") === "idle" && R.nextRecordState("saving", "saved") === "idle", "a failed or finished save ends in idle");
check(R.nextRecordState("recording", "failed") === "recording" && R.nextRecordState("paused", "saved") === "paused" && R.nextRecordState("idle", "failed") === "idle", "saved/failed mean nothing outside saving");
check(R.statusWord("idle", false, false, true) === "Starting…" && R.statusWord("idle", false, true, true) === "Starting…", "the disc tapped, the microphone opening: Starting…");
check(R.statusWord("recording", false, false, true) === "Recording" && R.statusWord("idle", false, false) === "Ready", "starting only shows while idle; the default is still Ready");

console.log(fails.length ? `${fails.length} FAILED` : "ALL PASS");
process.exit(fails.length ? 1 : 0);
