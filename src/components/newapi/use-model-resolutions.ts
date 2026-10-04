import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "@/lib/api";
import type { ProviderModelResolution, ProviderModelResolutionDraft } from "@/types/newapi";

export interface ModelResolutionState {
  models: ReadonlyMap<string, ProviderModelResolution>;
  loading: boolean;
  error: boolean;
  retry: () => void;
}

// Only this adapter knows about IPC. The editor receives read-only summaries,
// and never copies resolved values into its saved manual overrides.
export function useModelResolutions(open: boolean, draft: ProviderModelResolutionDraft | null): ModelResolutionState {
  const [retryVersion, setRetryVersion] = useState(0);
  const [result, setResult] = useState<{ scope: string; models: ProviderModelResolution[]; error: boolean } | null>(null);
  const session = useRef({ open, generation: 0 });
  if (session.current.open !== open) {
    session.current = { open, generation: session.current.generation + 1 };
  }
  const draftKey = open && draft?.models.length ? JSON.stringify(draft) : "";
  const scope = draftKey ? `${session.current.generation}:${retryVersion}:${draftKey}` : "";
  const currentScope = useRef(scope);
  currentScope.current = scope;

  useEffect(() => {
    if (!draftKey) return;
    let active = true;
    const timer = setTimeout(() => {
      const snapshot: ProviderModelResolutionDraft = JSON.parse(draftKey);
      void api.providers.previewModelResolutions(snapshot).then(
        value => {
          if (active && currentScope.current === scope) setResult({ scope, models: value.models, error: false });
        },
        () => {
          if (active && currentScope.current === scope) setResult({ scope, models: [], error: true });
        },
      );
    }, 160);
    return () => { active = false; clearTimeout(timer); };
  }, [draftKey, scope]);

  const current = result?.scope === scope ? result : null;
  const models = useMemo(() => new Map((current?.models ?? []).map(model => [model.modelId, model])), [current]);
  return {
    models,
    loading: Boolean(scope && !current),
    error: Boolean(current?.error || (open && draft === null)),
    retry: () => setRetryVersion(value => value + 1),
  };
}
