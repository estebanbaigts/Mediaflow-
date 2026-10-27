import "./tauri-mock";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { calls, emit, onInvoke, resetMock } from "./tauri-mock";
import { detail, dstGrant, overview, plan, profile, session, srcGrant } from "./fixtures";
import type { GrantView } from "../types";

let grants: GrantView[];

beforeEach(() => {
  resetMock();
  grants = [];
  onInvoke("overview", () => overview());
  onInvoke("list_grants", () => grants);
  onInvoke("list_profiles", () => [profile]);
  onInvoke("pick_folder", (a) => {
    const g = a?.kind === "source" ? srcGrant : dstGrant;
    grants = [...grants.filter((x) => x.id !== g.id), g];
    return g;
  });
  onInvoke("create_plan", () => plan());
  onInvoke("discard_plan", () => null);
  onInvoke("start_import", () => "s-1");
  onInvoke("session_detail", () => detail("completed_with_errors"));
  onInvoke("resume_import", () => null);
  onInvoke("cancel_import", () => null);
  onInvoke("list_sessions", () => []);
});

async function goToImport(user: ReturnType<typeof userEvent.setup>) {
  render(<App />);
  await user.click(within(screen.getByRole("navigation", { name: "Main" })).getByRole("button", { name: /New import/ }));
}

describe("navigation", () => {
  it("moves through the sidebar with the keyboard and shortcuts", async () => {
    const user = userEvent.setup();
    render(<App />);
    const nav = screen.getByRole("navigation", { name: "Main" });
    const first = within(nav).getAllByRole("button")[0]!;
    first.focus();
    await user.keyboard("{ArrowDown}");
    expect(within(nav).getAllByRole("button")[1]).toHaveFocus();
    await user.keyboard("{Control>}7{/Control}");
    expect(await screen.findByRole("heading", { name: "History" })).toBeInTheDocument();
    expect(within(nav).getByRole("button", { name: /History/ })).toHaveAttribute("aria-current", "page");
  });
});

describe("new import flow", () => {
  it("goes from source to a verified run and never reports problems as success", async () => {
    const user = userEvent.setup();
    await goToImport(user);

    await user.click(screen.getByRole("button", { name: /Choose a card or folder/ }));
    expect(await screen.findByLabelText("Source name")).toHaveValue("A7S3");
    await user.click(screen.getByRole("button", { name: "Continue" }));

    expect(screen.getByRole("button", { name: "Analyze" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: /Add a destination folder/ }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Analyze" })).toBeEnabled());
    await user.type(screen.getByLabelText("Project name"), "Launch");
    await user.click(screen.getByRole("button", { name: "Analyze" }));

    // Preview: exact counts, conflicts and exclusions are visible.
    expect(await screen.findByText("conflicts, not copied")).toBeInTheDocument();
    const createCall = calls.find((c) => c.cmd === "create_plan")!;
    expect(createCall.args).toEqual({
      args: { source_grant: "g-src", destination_grants: ["g-dst"], profile_id: "youtube", project: "Launch", label: "A7S3", quick: false },
    });
    // No raw path is ever sent to the backend.
    expect(JSON.stringify(calls.map((c) => c.args))).not.toContain("/Volumes");

    await user.click(screen.getByRole("tab", { name: /Conflicts/ }));
    expect(screen.getByText("C0003.MP4")).toBeInTheDocument();
    expect(screen.queryByText("C0001.MP4")).not.toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: /Excluded/ }));
    expect(screen.getByText(/macOS folder metadata/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Copy and verify 1 file/ }));
    expect(calls.some((c) => c.cmd === "start_import" && c.args?.planId === "plan-1")).toBe(true);

    // Live progress from engine events.
    act(() =>
      emit("import-progress", {
        session_id: "s-1",
        state: "copying",
        ops_total: 3,
        ops_done: 0,
        ops_failed: 0,
        ops_skipped: 1,
        bytes_to_copy: 2000,
        bytes_copied: 1000,
        bytes_to_verify: 2000,
        bytes_verified: 0,
        fraction: 0.25,
        bytes_per_second: null,
        eta_seconds: null,
        current: { file_id: 0, rel_path: "C0001.MP4", size: 2000, phase: "copying", done: 1000 },
        last_outcome: null,
      }),
    );
    expect(await screen.findByText("25")).toBeInTheDocument();
    expect(screen.getByText("Measuring speed…")).toBeInTheDocument(); // no fake ETA
    expect(screen.getByRole("progressbar", { name: "Overall progress" })).toHaveAttribute("aria-valuenow", "25");

    act(() => emit("import-finished", { session_id: "s-1", state: "completed_with_errors" }));
    expect(await screen.findByText(/Finished with problems — do not format the card yet/)).toBeInTheDocument();
    expect(screen.queryByText(/Import complete/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry failed" })).toBeInTheDocument();
  });

  it("blocks the start when a destination lacks space", async () => {
    onInvoke("create_plan", () =>
      plan({
        destinations: [{ root: "/x", label: "Small", required_bytes: 10e9, available_bytes: 1e9, space_ok: false, existing_files: 0, index_complete: true }],
      }),
    );
    grants = [srcGrant, dstGrant];
    const user = userEvent.setup();
    await goToImport(user);
    await user.click(await screen.findByRole("radio", { name: /A7S3/ }));
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await user.click(screen.getByRole("checkbox", { name: /Projects/ }));
    await user.click(screen.getByRole("button", { name: "Analyze" }));
    expect(await screen.findByText("Not enough space")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Copy and verify/ })).toBeDisabled();
  });

  it("shows engine errors in plain language", async () => {
    onInvoke("create_plan", () => {
      throw { code: "not_authorized", message: "MediaFlow is not allowed to access this folder." };
    });
    grants = [srcGrant, dstGrant];
    const user = userEvent.setup();
    await goToImport(user);
    await user.click(await screen.findByRole("radio", { name: /A7S3/ }));
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await user.click(screen.getByRole("checkbox", { name: /Projects/ }));
    await user.click(screen.getByRole("button", { name: "Analyze" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("not allowed to access");
  });

  it("disconnected folders cannot be selected", async () => {
    grants = [{ ...srcGrant, available: false }];
    const user = userEvent.setup();
    await goToImport(user);
    const radio = await screen.findByRole("radio", { name: /A7S3/ });
    expect(radio).toBeDisabled();
    expect(screen.getByText("Not connected")).toBeInTheDocument();
  });
});

describe("overview", () => {
  it("offers resume for an interrupted import", async () => {
    onInvoke("overview", () => overview({ attention: [session("interrupted", { last_error: "\"/Volumes/RAID\" is no longer available." })] }));
    onInvoke("session_detail", () => detail("interrupted"));
    const user = userEvent.setup();
    render(<App />);
    expect(await screen.findByText(/no longer available/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Resume" }));
    expect(calls.some((c) => c.cmd === "resume_import" && c.args?.sessionId === "s-1")).toBe(true);
  });

  it("explains an empty state", async () => {
    render(<App />);
    expect(await screen.findByText("No imports yet")).toBeInTheDocument();
  });
});

describe("cancel", () => {
  it("requires confirmation", async () => {
    onInvoke("overview", () => overview({ attention: [session("paused")] }));
    onInvoke("session_detail", () => detail("paused"));
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Details" }));
    await user.click(await screen.findByRole("button", { name: "Cancel import" }));
    expect(calls.some((c) => c.cmd === "cancel_import")).toBe(false);
    expect(screen.getByText(/A cancelled import cannot be resumed/)).toBeInTheDocument();
    const dialog = screen.getByRole("dialog", { hidden: true });
    await user.click(within(dialog).getByRole("button", { name: "Cancel import", hidden: true }));
    expect(calls.some((c) => c.cmd === "cancel_import" && c.args?.sessionId === "s-1")).toBe(true);
  });
});
