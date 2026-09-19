#!/usr/bin/env node

import { execFileSync, spawn } from "node:child_process";
import { mkdir, open, readdir, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

const DEFAULT_SAMPLE_RATE = 48_000;
const DEFAULT_CHANNELS = 2;
const DEFAULT_CHUNK_SECONDS = 60;
const BITS_PER_SAMPLE = 16;

export function parseSourceList(output) {
  return output
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line) => {
      const [id, name, driver, ...descriptionParts] = line.split("\t");
      return { id, name, driver, description: descriptionParts.join("\t") };
    });
}

export function parseArguments(argv) {
  const [command = "help", ...rest] = argv;
  const options = {};
  for (let index = 0; index < rest.length; index += 1) {
    const token = rest[index];
    if (!token.startsWith("--")) throw new Error(`Unexpected argument: ${token}`);
    const key = token.slice(2);
    if (["consent-confirmed", "echo-cancel"].includes(key)) {
      options[toCamelCase(key)] = true;
      continue;
    }
    const value = rest[index + 1];
    if (!value || value.startsWith("--")) throw new Error(`Missing value for --${key}`);
    index += 1;
    options[toCamelCase(key)] = value;
  }
  return { command, options };
}

export function buildParecArguments({ source, sampleRate, channels }) {
  return [
    `--device=${source}`,
    "--format=s16le",
    `--rate=${sampleRate}`,
    `--channels=${channels}`,
    "--raw",
  ];
}

export function createWavHeader({ dataBytes, sampleRate, channels }) {
  const bytesPerSample = BITS_PER_SAMPLE / 8;
  const blockAlign = channels * bytesPerSample;
  const header = Buffer.alloc(44);
  header.write("RIFF", 0, "ascii");
  header.writeUInt32LE(36 + dataBytes, 4);
  header.write("WAVE", 8, "ascii");
  header.write("fmt ", 12, "ascii");
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(channels, 22);
  header.writeUInt32LE(sampleRate, 24);
  header.writeUInt32LE(sampleRate * blockAlign, 28);
  header.writeUInt16LE(blockAlign, 32);
  header.writeUInt16LE(BITS_PER_SAMPLE, 34);
  header.write("data", 36, "ascii");
  header.writeUInt32LE(dataBytes, 40);
  return header;
}

export class WavChunkWriter {
  constructor({ directory, track, sampleRate, channels, chunkSeconds }) {
    this.directory = directory;
    this.track = track;
    this.sampleRate = sampleRate;
    this.channels = channels;
    this.blockAlign = channels * (BITS_PER_SAMPLE / 8);
    this.bytesPerChunk = sampleRate * this.blockAlign * chunkSeconds;
    this.handle = null;
    this.dataBytes = 0;
    this.index = 0;
    this.chunks = [];
    this.pending = Buffer.alloc(0);
    this.totalDataBytes = 0;
  }

  async write(input) {
    let buffer = this.pending.length ? Buffer.concat([this.pending, input]) : input;
    const alignedLength = buffer.length - (buffer.length % this.blockAlign);
    this.pending = buffer.subarray(alignedLength);
    buffer = buffer.subarray(0, alignedLength);
    let offset = 0;
    while (offset < buffer.length) {
      if (!this.handle) await this.openChunk();
      const length = Math.min(this.bytesPerChunk - this.dataBytes, buffer.length - offset);
      await this.handle.write(buffer, offset, length, 44 + this.dataBytes);
      this.dataBytes += length;
      this.totalDataBytes += length;
      offset += length;
      await this.updateHeader();
      if (this.dataBytes === this.bytesPerChunk) await this.finalizeChunk();
    }
  }

  async close() {
    if (this.pending.length) {
      const padded = Buffer.alloc(this.blockAlign);
      this.pending.copy(padded);
      this.pending = Buffer.alloc(0);
      await this.write(padded);
    }
    await this.finalizeChunk();
    return [...this.chunks];
  }

  async openChunk() {
    const filename = `${this.track}-${String(this.index).padStart(5, "0")}.wav`;
    this.handle = await open(path.join(this.directory, filename), "wx", 0o600);
    this.dataBytes = 0;
    this.currentFilename = filename;
    await this.updateHeader();
  }

  async updateHeader() {
    const header = createWavHeader({
      dataBytes: this.dataBytes,
      sampleRate: this.sampleRate,
      channels: this.channels,
    });
    await this.handle.write(header, 0, header.length, 0);
  }

  async finalizeChunk() {
    if (!this.handle) return;
    await this.updateHeader();
    await this.handle.sync();
    await this.handle.close();
    this.chunks.push(this.currentFilename);
    this.handle = null;
    this.currentFilename = null;
    this.dataBytes = 0;
    this.index += 1;
  }
}

function toCamelCase(value) {
  return value.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
}

function runPactl(args) {
  try {
    return execFileSync("pactl", args, { encoding: "utf8" }).trim();
  } catch (error) {
    const detail = error.stderr?.toString().trim() || error.message;
    throw new Error(`Unable to run pactl ${args[0]}: ${detail}`);
  }
}

export function discoverAudioDevices(run = runPactl) {
  const sources = parseSourceList(run(["list", "short", "sources"]));
  const defaultSource = run(["get-default-source"]);
  const defaultSink = run(["get-default-sink"]);
  const expectedMonitor = `${defaultSink}.monitor`;
  const systemSource = sources.find((source) => source.name === expectedMonitor)
    ?? sources.find((source) => source.name.endsWith(".monitor"));
  return { defaultSource, defaultSink, systemSource: systemSource?.name ?? null, sources };
}

function parseShortRows(output) {
  return output.split(/\r?\n/).filter(Boolean).map((line) => line.split("\t"));
}

function enableWebRtcEchoCancellation(devices, run = runPactl) {
  const token = process.pid;
  const sourceName = `transcrip_it_aec_source_${token}`;
  const sinkName = `transcrip_it_aec_sink_${token}`;
  const sinksBefore = new Map(parseShortRows(run(["list", "short", "sinks"]))
    .map(([id, name]) => [id, name]));
  const inputsBefore = new Map(parseShortRows(run(["list", "short", "sink-inputs"]))
    .map(([id, sinkId]) => [id, sinksBefore.get(sinkId) ?? devices.defaultSink]));
  const moduleId = run([
    "load-module",
    "module-echo-cancel",
    `source_name=${sourceName}`,
    `sink_name=${sinkName}`,
    `source_master=${devices.defaultSource}`,
    `sink_master=${devices.defaultSink}`,
    "aec_method=webrtc",
    "format=s16le",
    "rate=48000",
    "channels=1",
    "channel_map=mono",
  ]);

  let routed = false;
  try {
    const sources = new Set(parseShortRows(run(["list", "short", "sources"]))
      .map(([, name]) => name));
    const sinks = new Map(parseShortRows(run(["list", "short", "sinks"]))
      .map(([id, name]) => [id, name]));
    if (!sources.has(sourceName) || !sources.has(`${sinkName}.monitor`) || ![...sinks.values()].includes(sinkName)) {
      throw new Error("PulseAudio did not expose the expected WebRTC virtual devices.");
    }

    run(["set-default-sink", sinkName]);
    for (const inputId of inputsBefore.keys()) run(["move-sink-input", inputId, sinkName]);
    routed = true;
  } catch (error) {
    try { run(["set-default-sink", devices.defaultSink]); } catch {}
    try { run(["unload-module", moduleId]); } catch {}
    throw error;
  }

  return {
    sourceName,
    sinkName,
    moduleId,
    cleanup() {
      const failures = [];
      try { run(["set-default-sink", devices.defaultSink]); } catch (error) { failures.push(error.message); }
      if (routed) {
        try {
          const currentSinks = new Map(parseShortRows(run(["list", "short", "sinks"]))
            .map(([id, name]) => [id, name]));
          for (const [inputId, sinkId] of parseShortRows(run(["list", "short", "sink-inputs"]))) {
            const currentSink = currentSinks.get(sinkId);
            const destination = inputsBefore.get(inputId)
              ?? (currentSink === sinkName ? devices.defaultSink : null);
            if (destination) run(["move-sink-input", inputId, destination]);
          }
        } catch (error) {
          failures.push(error.message);
        }
      }
      try { run(["unload-module", moduleId]); } catch (error) { failures.push(error.message); }
      return failures;
    },
  };
}

function positiveInteger(value, fallback, label) {
  if (value === undefined) return fallback;
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed <= 0) throw new Error(`${label} must be a positive integer.`);
  return parsed;
}

function timestamp() {
  return new Date().toISOString().replace(/[:.]/g, "-");
}

async function writeJsonAtomic(file, value) {
  const temporary = `${file}.tmp`;
  await writeFile(temporary, `${JSON.stringify(value, null, 2)}\n`, { encoding: "utf8", mode: 0o600 });
  await rename(temporary, file);
}

function printDevices(devices) {
  console.log(`Default microphone: ${devices.defaultSource}`);
  console.log(`Default output:     ${devices.defaultSink}`);
  console.log(`System monitor:     ${devices.systemSource ?? "not found"}`);
  console.log("\nAvailable recording sources:");
  for (const source of devices.sources) {
    const roles = [];
    if (source.name === devices.defaultSource) roles.push("default microphone");
    if (source.name === devices.systemSource) roles.push("system audio");
    console.log(`- ${source.name}${roles.length ? ` (${roles.join(", ")})` : ""}`);
  }
}

function startCapture({ track, directory, sampleRate, channels, chunkSeconds }) {
  const writer = new WavChunkWriter({ directory, track: track.name, sampleRate, channels, chunkSeconds });
  const child = spawn("parec", buildParecArguments({
    source: track.source,
    sampleRate,
    channels,
  }), { stdio: ["ignore", "pipe", "pipe"] });
  let diagnostics = "";
  child.stderr.on("data", (chunk) => {
    diagnostics += chunk.toString();
    if (diagnostics.length > 16_384) diagnostics = diagnostics.slice(-16_384);
  });
  let firstSampleAt = null;
  const completion = new Promise((resolve) => {
    let settled = false;
    const settle = (result) => {
      if (settled) return;
      settled = true;
      resolve(result);
    };
    child.once("error", (error) => settle({ error: error.message }));
    child.once("exit", (code, signal) => settle({ code, signal }));
  });
  const reading = (async () => {
    for await (const chunk of child.stdout) {
      firstSampleAt ??= new Date().toISOString();
      await writer.write(chunk);
    }
  })();
  return {
    track,
    child,
    writer,
    completion,
    reading,
    firstSampleAt: () => firstSampleAt,
    diagnostics: () => diagnostics.trim(),
  };
}

async function record(options) {
  if (!options.consentConfirmed) throw new Error("Recording requires --consent-confirmed.");
  const mode = options.mode ?? "both";
  if (!["mic", "system", "both"].includes(mode)) throw new Error("--mode must be mic, system, or both.");

  const sampleRate = positiveInteger(options.sampleRate, DEFAULT_SAMPLE_RATE, "--sample-rate");
  const channels = positiveInteger(options.channels, options.echoCancel ? 1 : DEFAULT_CHANNELS, "--channels");
  if (options.echoCancel && mode !== "both") throw new Error("--echo-cancel currently requires --mode both.");
  if (options.echoCancel && sampleRate !== 48_000) throw new Error("--echo-cancel requires --sample-rate 48000.");
  if (options.echoCancel && channels !== 1) throw new Error("--echo-cancel requires --channels 1.");
  if (options.echoCancel) {
    console.warn("MVP audio notice: a headset is recommended for best separation.");
    console.warn("Loud system playback during simultaneous speech may suppress the local microphone.");
  }
  const chunkSeconds = positiveInteger(options.chunkSeconds, DEFAULT_CHUNK_SECONDS, "--chunk-seconds");
  const duration = options.duration === undefined
    ? undefined
    : positiveInteger(options.duration, undefined, "--duration");
  const devices = discoverAudioDevices();
  const echoSession = options.echoCancel ? enableWebRtcEchoCancellation(devices) : null;
  try {
  const requestedTracks = [];
  if (mode === "mic" || mode === "both") {
    requestedTracks.push({
      name: "mic",
      source: echoSession?.sourceName ?? options.micSource ?? devices.defaultSource,
    });
    if (echoSession) {
      requestedTracks.push({ name: "mic_raw", source: options.micSource ?? devices.defaultSource });
    }
  }
  if (mode === "system" || mode === "both") {
    const source = echoSession ? `${echoSession.sinkName}.monitor` : options.systemSource ?? devices.systemSource;
    if (!source) throw new Error("No system-audio monitor source was found. Pass --system-source explicitly.");
    requestedTracks.push({ name: "system", source });
  }

  const sessionDirectory = path.resolve(options.output ?? path.join("recordings", timestamp()));
  await mkdir(sessionDirectory, { recursive: true, mode: 0o700 });
  if ((await readdir(sessionDirectory)).length) throw new Error(`Output directory must be empty: ${sessionDirectory}`);

  const manifestPath = path.join(sessionDirectory, "manifest.json");
  const manifest = {
    schemaVersion: 1,
    status: "recording",
    startedAt: new Date().toISOString(),
    stoppedAt: null,
    sampleRate,
    channels,
    chunkSeconds,
    requestedDurationSeconds: duration ?? null,
    echoCancellation: echoSession ? {
      enabled: true,
      method: "pulseaudio-webrtc",
      processingFormat: "s16le 1ch 48000Hz",
      rawMicrophonePreserved: true,
      headsetRecommended: true,
      knownLimitation: "Loud simultaneous system playback may suppress local speech.",
    } : { enabled: false },
    tracks: requestedTracks.map((track) => ({ ...track, chunks: [] })),
    errors: [],
  };
  await writeJsonAtomic(manifestPath, manifest);
  console.log(`Recording ${mode} audio to ${sessionDirectory}`);
  console.log("Press Ctrl+C to stop safely.");

  let stopReason = null;
  const captures = requestedTracks.map((track) => startCapture({
    track, directory: sessionDirectory, sampleRate, channels, chunkSeconds,
  }));
  const stopChildren = (reason) => {
    if (stopReason) return;
    stopReason = reason;
    for (const { child } of captures) if (!child.killed) child.kill("SIGINT");
  };
  const handleInterrupt = () => stopChildren("interrupt");
  const handleTermination = () => stopChildren("termination");
  process.once("SIGINT", handleInterrupt);
  process.once("SIGTERM", handleTermination);
  const timer = duration ? setTimeout(() => stopChildren("duration"), duration * 1000) : null;

  const results = await Promise.all(captures.map(async (capture) => {
    let readError = null;
    try {
      await capture.reading;
    } catch (error) {
      readError = error;
      if (!capture.child.killed) capture.child.kill("SIGINT");
    }
    const processResult = await capture.completion;
    const chunks = await capture.writer.close();
    return {
      name: capture.track.name,
      chunks,
      firstSampleAt: capture.firstSampleAt(),
      capturedDurationSeconds: capture.writer.totalDataBytes / (sampleRate * channels * (BITS_PER_SAMPLE / 8)),
      ...processResult,
      error: readError?.message ?? processResult.error,
      diagnostics: capture.diagnostics(),
    };
  }));

  if (timer) clearTimeout(timer);
  process.removeListener("SIGINT", handleInterrupt);
  process.removeListener("SIGTERM", handleTermination);
  for (const track of manifest.tracks) {
    const result = results.find((candidate) => candidate.name === track.name);
    track.chunks = result.chunks;
    track.firstSampleAt = result.firstSampleAt;
    track.capturedDurationSeconds = Number(result.capturedDurationSeconds.toFixed(3));
  }
  for (const result of results) {
    if (result.error || (!stopReason && result.code !== 0)) {
      manifest.errors.push({
        track: result.name,
        message: result.error ?? `parec exited with code ${result.code}`,
        diagnostics: result.diagnostics,
      });
    }
  }
  manifest.status = manifest.errors.length ? "failed" : "complete";
  manifest.stopReason = stopReason ?? "source-ended";
  manifest.stoppedAt = new Date().toISOString();
  await writeJsonAtomic(manifestPath, manifest);
  if (manifest.errors.length) throw new Error(`Capture failed. See ${manifestPath}`);
  console.log(`Capture complete. Manifest: ${manifestPath}`);
  } finally {
    if (echoSession) {
      const failures = echoSession.cleanup();
      if (failures.length) console.error(`Warning: audio routing cleanup reported: ${failures.join("; ")}`);
      else console.log("Original audio routing restored.");
    }
  }
}

function printHelp() {
  console.log(`Transcrip It audio-capture feasibility spike

Usage:
  node spikes/audio-capture/audio.mjs devices
  node spikes/audio-capture/audio.mjs record --consent-confirmed [options]

Record options:
  --mode mic|system|both       Capture mode (default: both)
  --output PATH                Empty session output directory
  --duration SECONDS           Stop automatically after a duration
  --chunk-seconds SECONDS      Recoverable WAV chunk length (default: 60)
  --sample-rate HZ             Output sample rate (default: 48000)
  --channels COUNT             Output channel count (default: 2)
  --mic-source NAME            Override the default microphone source
  --system-source NAME         Override the detected monitor source
  --echo-cancel               Route playback through WebRTC AEC during capture
  --consent-confirmed          Required acknowledgement before recording`);
}

export async function main(argv = process.argv.slice(2)) {
  const { command, options } = parseArguments(argv);
  if (command === "devices") printDevices(discoverAudioDevices());
  else if (command === "record") await record(options);
  else if (command === "help" || command === "--help" || command === "-h") printHelp();
  else throw new Error(`Unknown command: ${command}`);
}

const invokedDirectly = process.argv[1] && import.meta.url === new URL(`file://${path.resolve(process.argv[1])}`).href;
if (invokedDirectly) {
  main().catch((error) => {
    console.error(`Error: ${error.message}`);
    process.exitCode = 1;
  });
}
