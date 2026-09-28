import { useCallback, useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { storageKeys } from "@/lib/storage-keys";
import { loadSavedConfig, type SavedImageConfig } from "./workbench-types";

const IMAGE_MODEL = /image|dall|gpt-image/i;

/**
 * Provider choice, manual site/key credentials and the generation parameters that
 * persist between sessions. Keys never live in the browser; the backend stores them.
 */
export function useImageConnection(pageActive: boolean, setError: (error: string | null) => void) {
  const state = useQuery({ queryKey: queryKeys.providers, queryFn: api.providers.loadState, refetchOnWindowFocus: false });
  const providers = state.data?.providers ?? [];
  const imageProviders = useMemo(
    () =>
      providers.filter(
        provider =>
          provider.hasApiKey &&
          ((!!provider.model && IMAGE_MODEL.test(provider.model)) ||
            provider.models.some(model => IMAGE_MODEL.test(model.id))),
      ),
    [providers],
  );

  const [providerId, setProviderId] = useState<string>("");
  const savedConfig = useMemo(loadSavedConfig, []);
  const [legacyKeyMigrationPending, setLegacyKeyMigrationPending] = useState(
    () => Boolean(savedConfig.legacyApiKey),
  );
  const [providerChoiceInitialized, setProviderChoiceInitialized] = useState(
    () => Boolean(savedConfig.baseUrl || savedConfig.legacyApiKey),
  );
  const [baseUrl, setBaseUrl] = useState(savedConfig.baseUrl);
  const [apiKey, setApiKey] = useState(savedConfig.legacyApiKey ?? "");
  const [keyStored, setKeyStored] = useState(false);
  const [keySites, setKeySites] = useState<string[]>([]);
  const [model, setModel] = useState(savedConfig.model);
  const [size, setSize] = useState(savedConfig.size);
  const [quality, setQuality] = useState(savedConfig.quality);

  const selectedProvider = providers.find(provider => provider.id === providerId);
  const availableModels = selectedProvider?.models.filter(item => IMAGE_MODEL.test(item.id)) ?? [];
  const modelOptions = Array.from(new Set([
    "gpt-image-2",
    "gpt-image-2.5",
    ...availableModels.map(item => item.id),
  ]));

  useEffect(() => {
    if (!savedConfig.legacyApiKey) return;
    // Old builds kept the key in plain localStorage. Whatever happens next, the
    // plaintext copy must not survive this load.
    const scrubPlaintextKey = () => {
      try {
        localStorage.setItem(storageKeys.imageWorkbenchConfig, JSON.stringify({
          baseUrl: savedConfig.baseUrl, model: savedConfig.model,
          size: savedConfig.size, quality: savedConfig.quality,
        } satisfies SavedImageConfig));
      } catch {
        try { localStorage.removeItem(storageKeys.imageWorkbenchConfig); } catch { /* unavailable storage */ }
      }
      setLegacyKeyMigrationPending(false);
    };
    if (!savedConfig.baseUrl) {
      // No endpoint to bind the key to: keep it in the input for this session only.
      scrubPlaintextKey();
      return;
    }
    void api.image.saveKey(savedConfig.baseUrl, savedConfig.legacyApiKey)
      .then(() => {
        setKeyStored(true);
        setApiKey(current => current === savedConfig.legacyApiKey ? "" : current);
      })
      .catch(() => setError("image.errors.storage"))
      .finally(scrubPlaintextKey);
  }, [savedConfig]);

  useEffect(() => {
    if (!baseUrl.trim()) { setKeyStored(false); return; }
    if (!pageActive) return;
    let active = true;
    setKeyStored(false);
    const timeout = window.setTimeout(() => void api.image.keyStatus(baseUrl.trim()).then(stored => {
      if (active) setKeyStored(stored);
    }).catch(() => {
      if (active) setKeyStored(false);
    }), 350);
    return () => { active = false; window.clearTimeout(timeout); };
  }, [baseUrl, pageActive]);

  useEffect(() => {
    if (providerId || legacyKeyMigrationPending) return;
    try {
      localStorage.setItem(
        storageKeys.imageWorkbenchConfig,
        JSON.stringify({ baseUrl, model, size, quality } satisfies SavedImageConfig),
      );
    } catch { /* storage may be disabled */ }
  }, [baseUrl, legacyKeyMigrationPending, model, providerId, quality, size]);

  const refreshKeySites = useCallback(() => {
    void api.image.keySites().then(setKeySites).catch(() => setKeySites([]));
  }, []);
  useEffect(() => {
    if (pageActive) refreshKeySites();
  }, [pageActive, refreshKeySites]);

  const saveKey = () => {
    const pending = apiKey.trim();
    if (!baseUrl.trim() || !pending) return;
    void api.image.saveKey(baseUrl, pending).then(() => {
      setLegacyKeyMigrationPending(false);
      setKeyStored(true);
      setApiKey(current => current === pending ? "" : current);
      refreshKeySites();
    }).catch(() => setError("image.errors.storage"));
  };

  const forgetKey = () => {
    const site = baseUrl.trim();
    if (!site) return;
    void api.image.deleteKey(site).then(() => {
      setKeyStored(false);
      refreshKeySites();
    }).catch(() => setError("image.errors.storage"));
  };

  /** A manual key used by a successful generation is now stored by the backend. */
  const markKeyUsed = (usedKey: string) => {
    setKeyStored(true);
    setApiKey(current => current === usedKey ? "" : current);
    refreshKeySites();
  };

  useEffect(() => {
    if (providerChoiceInitialized || !state.isSuccess) return;
    setProviderChoiceInitialized(true);
    if (baseUrl || apiKey || imageProviders.length === 0) return;
    const provider = imageProviders[0];
    setProviderId(provider.id);
    setModel(
      provider.models.find(item => IMAGE_MODEL.test(item.id))?.id ||
        provider.model ||
        "gpt-image-2",
    );
  }, [apiKey, baseUrl, imageProviders, providerChoiceInitialized, state.isSuccess]);

  const chooseProvider = (id: string) => {
    setProviderChoiceInitialized(true);
    setProviderId(id);
    const provider = providers.find(item => item.id === id);
    if (provider) {
      setModel(
        provider.models.find(item => IMAGE_MODEL.test(item.id))?.id ||
          (IMAGE_MODEL.test(provider.model) ? provider.model : "gpt-image-2"),
      );
    }
  };

  const hasCredentials = Boolean(providerId || (baseUrl.trim() && (apiKey.trim() || keyStored)));

  return {
    imageProviders, providerId, chooseProvider,
    baseUrl, setBaseUrl, apiKey, setApiKey, keyStored, keySites, saveKey, forgetKey, markKeyUsed,
    model, setModel, modelOptions, size, setSize, quality, setQuality, hasCredentials,
  };
}

export type ImageConnection = ReturnType<typeof useImageConnection>;
