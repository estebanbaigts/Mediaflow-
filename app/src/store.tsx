// Global state for running imports: progress events arrive here whatever page
// is visible, so leaving the run screen never loses the real state.
import { createContext, useContext, useEffect, useMemo, useReducer, type ReactNode } from "react";
import { events } from "./api";
import type { ImportFinished, Progress } from "./types";

export interface RunState {
  progress: Progress | null;
  finished: ImportFinished | null;
  /** Most recent per-file outcomes (newest first), errors kept. */
  outcomes: NonNullable<Progress["last_outcome"]>[];
}

type Action = { type: "progress"; p: Progress } | { type: "finished"; f: ImportFinished } | { type: "reset"; id: string };

type State = Record<string, RunState>;

function reducer(state: State, a: Action): State {
  switch (a.type) {
    case "progress": {
      const prev = state[a.p.session_id] ?? { progress: null, finished: null, outcomes: [] };
      let outcomes = prev.outcomes;
      const o = a.p.last_outcome;
      if (o && !(outcomes[0] && outcomes[0].file_id === o.file_id && outcomes[0].dest_idx === o.dest_idx && outcomes[0].state === o.state)) {
        outcomes = [o, ...outcomes].slice(0, 200);
      }
      return { ...state, [a.p.session_id]: { progress: a.p, finished: null, outcomes } };
    }
    case "finished": {
      const prev = state[a.f.session_id] ?? { progress: null, finished: null, outcomes: [] };
      return { ...state, [a.f.session_id]: { ...prev, finished: a.f } };
    }
    case "reset": {
      const next = { ...state };
      delete next[a.id];
      return next;
    }
  }
}

interface Ctx {
  runs: State;
  reset: (id: string) => void;
}

const RunsContext = createContext<Ctx>({ runs: {}, reset: () => {} });

export function RunsProvider({ children }: { children: ReactNode }) {
  const [runs, dispatch] = useReducer(reducer, {});
  useEffect(() => {
    let alive = true;
    const unsubs: (() => void)[] = [];
    void events.onImportProgress((p) => dispatch({ type: "progress", p })).then((u) => (alive ? unsubs.push(u) : u()));
    void events.onImportFinished((f) => dispatch({ type: "finished", f })).then((u) => (alive ? unsubs.push(u) : u()));
    return () => {
      alive = false;
      unsubs.forEach((u) => u());
    };
  }, []);
  const value = useMemo(() => ({ runs, reset: (id: string) => dispatch({ type: "reset", id }) }), [runs]);
  return <RunsContext.Provider value={value}>{children}</RunsContext.Provider>;
}

export function useRuns() {
  return useContext(RunsContext);
}

export function useRun(id: string | null): RunState | null {
  const { runs } = useRuns();
  return id ? (runs[id] ?? null) : null;
}
