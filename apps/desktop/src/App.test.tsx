import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import App from "./App";

describe("desktop shell", () => {
  it("presents the local meeting workspace", () => {
    render(<App />);

    expect(screen.getByRole("heading", { name: "Good morning" })).toBeInTheDocument();
    expect(screen.getByText("Use a headset for the clearest transcript")).toBeInTheDocument();
    expect(screen.getByText("Your meeting library starts here")).toBeInTheDocument();
  });

  it("explains the current recording milestone", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "New recording" }));
    expect(screen.getByRole("dialog", { name: "The workspace is ready" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Got it" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
