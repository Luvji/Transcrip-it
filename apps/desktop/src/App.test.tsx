import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import App, { latestLiveTranscriptLines, recordingWarningMessages } from "./App";

describe("desktop workspace", () => {
  beforeEach(() => localStorage.clear());

  it("presents the local meeting workspace", async () => {
    render(<App />);

    expect(screen.getByRole("heading", { name: "Good morning" })).toBeInTheDocument();
    expect(screen.getByText("Use a headset for the clearest transcript")).toBeInTheDocument();
    expect(await screen.findByText("Your meeting library starts here")).toBeInTheDocument();
  });

  it("requires consent and keeps browser preview from claiming audio capture", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "New recording" }));
    expect(screen.getByRole("heading", { name: "Prepare your meeting" })).toBeInTheDocument();
    const prepare = screen.getByRole("button", { name: "Start recording" });
    expect(prepare).toBeDisabled();

    await user.type(screen.getByRole("textbox", { name: "Meeting title" }), "Design review");
    await user.click(screen.getByRole("checkbox", { name: /I confirm everyone has consented/ }));
    expect(prepare).toBeDisabled();
    expect(screen.getByText(/Browser preview does not access your microphone/)).toBeInTheDocument();
  });

  it("saves local settings", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(screen.getAllByRole("button", { name: "Settings" })[0]);
    await user.selectOptions(screen.getByRole("combobox", { name: "Transcription quality" }), "balanced");
    await user.selectOptions(screen.getByRole("combobox", { name: "Microphone transcript source" }), "mic");
    await user.click(screen.getByRole("button", { name: "Save settings" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Settings saved"));
    expect(localStorage.getItem("transcrip-it.transcription-pack")).toBe("balanced");
    expect(localStorage.getItem("transcrip-it.microphone-track")).toBe("mic");
  });

  it("keeps the newest live transcript passages visible", () => {
    const lines = Array.from({ length: 5 }, (_, index) => ({
      trackId: "mic" as const,
      speakerLabel: "Microphone (original)",
      chunkIndex: 0,
      windowIndex: index,
      startMs: index * 8_000,
      text: `Passage ${index}`,
    }));

    expect(latestLiveTranscriptLines(lines).map((line) => line.startMs)).toEqual([
      16_000,
      24_000,
      32_000,
    ]);
  });

  it("promotes capture health failures into visible warning messages", () => {
    expect(recordingWarningMessages({
      active: true,
      meetingId: "meeting-1",
      elapsedSeconds: 12,
      processRunning: false,
      paused: false,
      levels: [],
      storageAvailableBytes: null,
      warnings: ["Microphone is muted."],
    })).toEqual([
      "Microphone is muted.",
      "Audio capture stopped unexpectedly. Stop the recording to preserve completed chunks.",
    ]);
  });
});
