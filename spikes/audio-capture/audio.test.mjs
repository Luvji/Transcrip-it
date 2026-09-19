import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

import {
  buildParecArguments,
  createWavHeader,
  discoverAudioDevices,
  parseArguments,
  pcmPeak,
  parseSourceList,
  WavChunkWriter,
} from "./audio.mjs";

test("measures normalized PCM peaks", () => {
  const samples = Buffer.alloc(6);
  samples.writeInt16LE(0, 0);
  samples.writeInt16LE(-16_384, 2);
  samples.writeInt16LE(32_767, 4);
  assert.ok(pcmPeak(samples) > 0.99);
  assert.equal(pcmPeak(Buffer.alloc(8)), 0);
});

test("parses PulseAudio source rows", () => {
  const sources = parseSourceList([
    "2\talsa_input.card.mic\tmodule-alsa-card.c\ts16le 2ch 44100Hz\tSUSPENDED",
    "4\talsa_output.card.monitor\tmodule-alsa-card.c\ts16le 2ch 44100Hz\tIDLE",
  ].join("\n"));
  assert.equal(sources.length, 2);
  assert.equal(sources[0].name, "alsa_input.card.mic");
  assert.equal(sources[1].name, "alsa_output.card.monitor");
});

test("discovers default microphone and matching monitor", () => {
  const responses = new Map([
    ["list short sources", "2\tmic\tdriver\tdetails\n4\tspeakers.monitor\tdriver\tdetails"],
    ["get-default-source", "mic"],
    ["get-default-sink", "speakers"],
  ]);
  const devices = discoverAudioDevices((args) => responses.get(args.join(" ")));
  assert.equal(devices.defaultSource, "mic");
  assert.equal(devices.systemSource, "speakers.monitor");
});

test("parses record options", () => {
  const parsed = parseArguments([
    "record", "--mode", "both", "--chunk-seconds", "30", "--echo-cancel", "--consent-confirmed",
  ]);
  assert.deepEqual(parsed, {
    command: "record",
    options: { mode: "both", chunkSeconds: "30", echoCancel: true, consentConfirmed: true },
  });
});

test("builds raw PulseAudio capture arguments", () => {
  assert.deepEqual(buildParecArguments({ source: "speakers.monitor", sampleRate: 48_000, channels: 2 }), [
    "--device=speakers.monitor",
    "--format=s16le",
    "--rate=48000",
    "--channels=2",
    "--raw",
  ]);
});

test("creates a valid PCM WAV header", () => {
  const header = createWavHeader({ dataBytes: 1_920, sampleRate: 48_000, channels: 2 });
  assert.equal(header.toString("ascii", 0, 4), "RIFF");
  assert.equal(header.toString("ascii", 8, 12), "WAVE");
  assert.equal(header.readUInt16LE(22), 2);
  assert.equal(header.readUInt32LE(24), 48_000);
  assert.equal(header.readUInt32LE(40), 1_920);
});

test("rotates WAV chunks at exact sample boundaries", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "transcrip-it-audio-"));
  try {
    const writer = new WavChunkWriter({
      directory, track: "mic", sampleRate: 10, channels: 1, chunkSeconds: 1,
    });
    await writer.write(Buffer.alloc(50, 1));
    const chunks = await writer.close();
    assert.deepEqual(chunks, ["mic-00000.wav", "mic-00001.wav", "mic-00002.wav"]);
    const sizes = [];
    for (const chunk of chunks) {
      const content = await readFile(path.join(directory, chunk));
      sizes.push(content.readUInt32LE(40));
    }
    assert.deepEqual(sizes, [20, 20, 10]);
    assert.equal(writer.totalDataBytes, 50);
  } finally {
    await rm(directory, { recursive: true });
  }
});
