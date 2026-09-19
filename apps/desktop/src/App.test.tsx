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

  it("requires consent and creates a local meeting", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "New recording" }));
    expect(screen.getByRole("heading", { name: "Prepare your meeting" })).toBeInTheDocument();
    const prepare = screen.getByRole("button", { name: "Prepare recording" });
    expect(prepare).toBeDisabled();

    await user.type(screen.getByRole("textbox", { name: "Meeting title" }), "Design review");
    await user.click(screen.getByRole("checkbox", { name: /I confirm everyone has consented/ }));
    await user.click(prepare);

    expect(await screen.findByRole("heading", { name: "Design review" })).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Meeting prepared locally");
  });

  it("saves local settings", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(screen.getAllByRole("button", { name: "Settings" })[0]);
    await user.click(screen.getByRole("button", { name: "Save settings" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Settings saved"));
  });
});
