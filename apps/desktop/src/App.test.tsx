import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import App from "./App";

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
    await user.click(screen.getByRole("button", { name: "Save settings" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Settings saved"));
    expect(localStorage.getItem("transcrip-it.transcription-pack")).toBe("balanced");
  });
});
