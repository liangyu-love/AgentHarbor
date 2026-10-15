import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { storageKeys } from "@/lib/storage-keys";
import { loadSavedConfig, qualitiesForModel, type SavedImageConfig } from "./workbench-types";

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
  const currentSite = useRef(baseUrl.trim());
  currentSite.current = baseUrl.trim();
  const keyStatusRequest = useRef(0);
  const [apiKey, setApiKey] = useState(savedConfig.legacyApiKey ?? "");
  const [keyStored, setKeyStored] = useState(false);
  const [keySites, setKeySites] = useState<string[]>([]);
  const [model, setModel] = useState(savedConfig.model);
  const [size, setSize] = useState(savedConfig.size);
  const [chosenQuality, setQuality] = useState(savedConfig.quality);
  const qualityOptions = qualitiesForModel(model);
  const quality = qualityOptions.some(item => item.value === chosenQuality) ? chosenQuality : "auto";

  const selectedProvider = providers.find(provider => provider.id === providerId);
  const availableModels = selectedProvider?.models.filter(item => IMAGE_MODEL.test(item.id)) ?? [];
  const modelOptions = Array.from(new Set([
    "gpt-image-2",
    "gpt-image-2.5-sunburst",
    "gpt-image-2.5-flare",
    ...availableModels.map(item => item.id),
  ]));

  const refreshKeyStatus = useCallback(async (site: string, usedKey?: string) => {
    if (site.trim() !== currentSite.current) return;
    const revision = ++keyStatusRequest.current;
    try {
      const stored = await api.image.keyStatus(site.trim());
      if (revision !== keyStatusRequest.current || site.trim() !== currentSite.current) return;
      setKeyStored(stored);
      if (stored && usedKey) setApiKey(current => current.trim() === usedKey.trim() ? "" : current);
    } catch {
      if (revision === keyStatusRequest.current && site.trim() === currentSite.current) setKeyStored(false);
    }
  }, []);

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
        void refreshKeyStatus(savedConfig.baseUrl, savedConfig.legacyApiKey);
      })
      .catch(() => setError("image.errors.storage"))
      .finally(scrubPlaintextKey);
  }, [savedConfig]);

  useEffect(() => {
    ++keyStatusRequest.current;
    if (!baseUrl.trim()) { setKeyStored(false); return; }
    if (!pageActive) return;
    setKeyStored(false);
    const timeout = window.setTimeout(() => void refreshKeyStatus(baseUrl), 350);
    return () => { ++keyStatusRequest.current; window.clearTimeout(timeout); };
  }, [baseUrl, pageActive, refreshKeyStatus]);

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
    const site = baseUrl.trim();
    if (!site || !pending) return;
    void api.image.saveKey(site, pending).then(() => {
      setLegacyKeyMigrationPending(false);
      void refreshKeyStatus(site, pending);
      refreshKeySites();
    }).catch(() => setError("image.errors.storage"));
  };

  const forgetKey = () => {
    const site = baseUrl.trim();
    if (!site) return;
    void api.image.deleteKey(site).then(() => {
      if (site === currentSite.current) {
        ++keyStatusRequest.current;
        setKeyStored(false);
      }
      refreshKeySites();
    }).catch(() => setError("image.errors.storage"));
  };

  /** A manual key used by a successful generation is now stored by the backend. */
  const markKeyUsed = (site: string, usedKey: string) => {
    void refreshKeyStatus(site, usedKey);
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
    model, setModel, modelOptions, size, setSize, quality, setQuality, qualityOptions, hasCredentials,
  };
}

export type ImageConnection = ReturnType<typeof useImageConnection>;
