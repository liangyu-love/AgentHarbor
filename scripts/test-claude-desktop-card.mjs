// A38: the Claude Desktop direct-connection card.
//
// The card is rendered by the shared frontend-regressions fixture
// (scripts/fixtures/frontend-regressions.tsx, mount("groups") -> <NewApiPage />), so it is
// measured inside the real page composition, directly after <ClientConfigs/>. That fixture
// stubs every api.* module the page touches except api.claudeDesktop, which still goes to the
// real Tauri IPC. This suite therefore installs a local fake IPC dispatcher on
// window.__TAURI_INTERNALS__.invoke - the same seam the main regression body and
// test-relay-deck.mjs already use - and answers the five claudeDesktop commands from synthetic
// DTOs. Long paths and long model ids are injected *through that seam*, so the overflow
// assertions below run against a real render instead of a class-name contract; the class
// contract itself (break-all on the config path, break-words on the model list) is asserted too.
import assert from "node:assert/strict";
import path from "node:path";

// Exact zh strings of src/locales/zh.json (the harness renders the app in Chinese, like its
// other suites). Keeping them literal means a renamed or dropped locale key fails here.
const ZH = {
  title: "Claude Desktop",
  clientsTitle: "客户端配置",
  stateManaged: "已配置",
  modeDirect: "AgentHarbor 直连",
  requiresRestart: "需要重启",
  restart: "重启 Claude Desktop",
  warningRequiresRestart: "请完全退出 Claude Desktop（包括托盘进程）后重新启动，设置才会生效。",
  change: "切换 / 更新",
  reapply: "重新应用",
  restorePrevious: "恢复接管前配置",
  switchOfficial: "切换官方模式",
  release: "释放管理",
  conflictReason: "请先解决冲突或按当前状态重新接管。",
  needModels: "请至少选择一个模型。",
  busyHint: "正在处理，请稍候。",
  dialogTitle: "配置 Claude Desktop",
  providerDisabled: "该卡片未加入路由池；直连不要求启用它，本次选择也不会启用。",
  noInferenceKey: "该卡片没有推理密钥；请先在卡片上补充，余额 token 不能作为推理凭据。",
  labelPlaceholder: "显示名称（可选）",
  oneMAuto: "添加",
  oneMOff: "不添加",
  messagesCheck: "核对 Messages 兼容性",
  preview: "预览",
  apply: "应用",
  cancel: "取消",
  confirm: "确认",
  restoreTitle: "恢复接管前的配置？",
  restoreDescription: "将撤回 AgentHarbor 写入的部署模式和配置，并恢复之前的生效配置选择；外部配置与无关键都会保留。",
  reapplyTitle: "重新应用配置？",
  reapplyDescription: "按已保存的模式与模型设置更新配置，接管前的还原点保持不变。",
  externalProfiles: "保留的外部配置",
  previewRestart: "应用后请完全退出并重新启动 Claude Desktop。",
};

const NORMAL_ROOT = "C:/Users/synthetic/AppData/Roaming/Claude";
const THIRD_PARTY_ROOT = "C:/Users/synthetic/AppData/Roaming/Claude-3p";
// Longer than any real profile: the card must break the path, never the page.
const LONG_PATH = NORMAL_ROOT + "/" + "nested-desktop-profile-folder/".repeat(9) + "claude_desktop_config.json";
const LONG_MODEL_ID = "claude-sonnet-4-5-20250929-" + "extended-context-preview-".repeat(4) + "direct";
const SHORT_MODEL_ID = "claude-sonnet-4-5";

const pathsSummary = {
  normalRoot: NORMAL_ROOT,
  thirdPartyRoot: THIRD_PARTY_ROOT,
  normalConfig: LONG_PATH,
  thirdPartyConfig: THIRD_PARTY_ROOT + "/claude_desktop_config.json",
  profilePath: NORMAL_ROOT + "/config.json",
  metaPath: NORMAL_ROOT + "/configLibrary/_meta.json",
  isolated: false,
};
const detection = { platform: "windows", version: "1.2.3", versionSource: "claudeDesktop.versionSources.package", installed: true, notes: [] };

const statusFixture = {
  detection,
  paths: pathsSummary,
  mode: "direct",
  managementState: "managed",
  isManaged: true,
  providerId: "group-a",
  providerName: "Relay Alpha",
  providerMissing: false,
  models: [{ modelId: LONG_MODEL_ID, label: LONG_MODEL_ID, oneM: true }, { modelId: SHORT_MODEL_ID, label: "", oneM: false }],
  modelChoices: [{ modelId: LONG_MODEL_ID, labelOverride: null, oneMMode: "auto" },
    { modelId: SHORT_MODEL_ID, labelOverride: null, oneMMode: "auto" }],
  staleReasons: [],
  conflicts: [],
  recoveryState: null,
  pendingRecovery: false,
  warnings: ["claudeDesktop.warnings.requiresRestart", "claudeDesktop.warnings.openPreviews", "claudeDesktop.warnings.isolated"],
  lastAppliedAt: 1_790_000_000,
  restartRequired: true,
  appRunning: false,
  restartCapable: false,
  launchMethod: null,
  actions: [
    { action: "applyDirect", enabled: true, disabledReason: null },
    { action: "applyRouter", enabled: true, disabledReason: null },
    { action: "reapply", enabled: true, disabledReason: null },
    { action: "restorePrevious", enabled: true, disabledReason: null },
    { action: "switchOfficial", enabled: false, disabledReason: "claudeDesktop.actionReasons.conflict" },
    { action: "release", enabled: true, disabledReason: null },
    { action: "recoverRollback", enabled: false, disabledReason: "claudeDesktop.actionReasons.noPending" },
  ],
  backupCount: 3,
};

const model = (modelId, extra) => ({ modelId, label: modelId, displayName: modelId, protocol: "anthropicMessages",
  protocolSource: "catalog", protocolVerified: true, included: true, supports1m: true, contextWindow: 1_000_000,
  contextSource: "catalog", selectable: true, excludeReason: null, ...extra });
const candidatesFixture = {
  paths: pathsSummary,
  pathCandidates: [{ id: "path-normal", normalRoot: NORMAL_ROOT, thirdPartyRoot: THIRD_PARTY_ROOT,
    normalExists: true, thirdPartyExists: true, hasInstallEvidence: true }],
  selectedPathId: "path-normal",
  detector: detection,
  providers: [
    { providerId: "group-a", name: "Relay Alpha", enabled: true, hasInferenceKey: true, eligible: true, reason: null,
      endpoint: { profileBaseUrl: "https://relay.invalid/v1", messagesUrl: "https://relay.invalid/v1/messages",
        modelsUrl: "https://relay.invalid/v1/models", plaintext: false },
      models: [
        model(LONG_MODEL_ID),
        model(SHORT_MODEL_ID, { protocolVerified: false, supports1m: false, contextWindow: 200_000 }),
        model("chat-only-model", { selectable: false, excludeReason: "claudeDesktop.errors.incompatibleProtocol" }),
      ] },
    { providerId: "group-b", name: "Relay Beta", enabled: false, hasInferenceKey: false, eligible: false,
      reason: "no_inference_key", endpoint: { profileBaseUrl: "https://beta.invalid", messagesUrl: "https://beta.invalid/v1/messages",
        modelsUrl: "https://beta.invalid/v1/models", plaintext: false }, models: [] },
  ],
  selectedProviderId: "group-a",
};

const managedModels = [{ modelId: LONG_MODEL_ID, label: LONG_MODEL_ID, oneM: true },
  { modelId: SHORT_MODEL_ID, label: SHORT_MODEL_ID, oneM: false }];
const previewFixture = {
  revision: "desktop-preview-1",
  action: "applyDirect",
  providerId: "group-a",
  providerName: "Relay Alpha",
  models: managedModels,
  messagesUrl: "https://relay.invalid/v1/messages",
  modelsUrl: "https://relay.invalid/v1/models",
  files: [
    { role: "normalConfig", path: LONG_PATH, kind: "update",
      managedChanges: [{ path: "deploymentMode", before: "1p", after: "3p" }], note: null },
    { role: "thirdPartyConfig", path: THIRD_PARTY_ROOT + "/claude_desktop_config.json", kind: "create",
      managedChanges: [{ path: "deploymentMode", before: null, after: "3p" }], note: null },
    { role: "profile", path: THIRD_PARTY_ROOT + "/configLibrary/profile.json", kind: "update",
      managedChanges: [{ path: "inferenceModels", before: '["claude-opus-4-6"]', after: null },
        { path: "inferenceGatewayApiKey", before: "<redacted>", after: "<redacted>" }], note: null },
  ],
  warnings: ["claudeDesktop.warnings.requiresRestart"],
  conflicts: [],
  requiresRestart: true,
  externalProfileCount: 1,
  managedEntryPresent: true,
  currentAppliedId: null,
  appliesTo: "Claude Desktop",
};
const appliedFixture = { transactionId: "desktop-tx-1", changedPaths: [LONG_PATH], backupId: "desktop-backup-1",
  managementState: "managed", requiresRestart: true, warnings: [] };
const probeFixture = { bearer: { providerId: "group-a", url: "https://relay.invalid/v1/models", status: "modelsAccessible",
  httpStatus: 200, egress: "direct", modelsVisible: [LONG_MODEL_ID], modelsMissing: [], listedModels: 1,
  checkedAt: 1_790_000_000, fingerprint: "fixture", note: null }, messages: null };

export async function testClaudeDesktopCard({ page, reset, captureDir }) {
  const card = () => page.locator('section[aria-labelledby="claude-desktop-title"]');
  const dialog = () => page.getByRole("dialog");
  const scroll = () => dialog().locator("[data-claude-desktop-scroll]");
  const modelCheckbox = modelId => page.locator('[id="desktop-model-' + modelId + '"]');
  const actionButton = name => card().getByRole("button", { name, exact: true });
  const focusedId = () => page.evaluate(() => document.activeElement?.id ?? "");
  // The dialog traps focus, so repeated Tab presses cycle through every control: stop when the
  // wanted id owns focus, fail the caller when it never does.
  const tabTo = async id => {
    for (let step = 0; step < 90; step++) {
      if (await focusedId() === id) return true;
      await page.keyboard.press("Tab");
    }
    return false;
  };

  const install = () => page.evaluate(fixture => {
    const review = window.review;
    const clone = value => value === undefined ? null : JSON.parse(JSON.stringify(value));
    review.desktopStatus = fixture.status;
    review.desktopCandidates = fixture.candidates;
    review.desktopPreview = fixture.preview;
    review.desktopApplied = fixture.applied;
    review.desktopProbe = fixture.probe;
    review.desktopCalls = { status: 0, candidates: [], preview: [], commit: [], probe: [], cancelPreview: [], restart: 0, other: [] };
    review.desktopHoldPreview = false;
    review.desktopPreviewResolvers = [];
    review.desktopPreviewFailure = null;
    review.desktopHoldProbe = false;
    review.desktopProbeResolvers = [];
    review.desktopProbeFailure = null;
    review.desktopCommitFailure = null;
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "get_claude_desktop_status") { review.desktopCalls.status++; return clone(review.desktopStatus); }
      if (command === "list_claude_desktop_candidates") {
        review.desktopCalls.candidates.push(clone(args));
        return clone(review.desktopCandidates);
      }
      if (command === "preview_claude_desktop_change") {
        review.desktopCalls.preview.push(clone(args));
        if (review.desktopPreviewFailure) throw review.desktopPreviewFailure;
        const preview = clone(review.desktopPreview);
        preview.action = args.action;
        preview.router = args.action === "applyRouter"
          ? { boundPort: 43127, routeRevision: 2, gatewayTokenVersion: 1, listenerReady: true } : null;
        const provider = review.desktopCandidates.providers.find(entry => entry.providerId === args.request.providerId);
        preview.providerName = provider?.name ?? preview.providerName;
        preview.messagesUrl = provider?.endpoint?.messagesUrl ?? preview.messagesUrl;
        preview.models = args.action === "applyRouter" ? (args.request.routes ?? []).map(choice => {
          const target = review.desktopCandidates.routeProviders.find(entry => entry.providerId === choice.providerId);
          const candidate = target?.models.find(entry => entry.modelId === choice.modelId);
          return { modelId: choice.roleAlias, label: choice.labelOverride ?? `${target?.name} · ${candidate?.label ?? choice.modelId}`,
            oneM: choice.oneMMode === "auto" && !!candidate?.supports1m };
        }) : (args.request.models ?? []).map(choice => {
          const candidate = provider?.models.find(entry => entry.modelId === choice.modelId);
          return { modelId: choice.modelId, label: choice.labelOverride ?? candidate?.label ?? choice.modelId,
            oneM: choice.oneMMode === "auto" && !!candidate?.supports1m };
        });
        for (const file of preview.files) {
          for (const change of file.managedChanges) {
            if (change.path === "inferenceModels") {
              change.after = JSON.stringify(preview.models.map(model => {
                const value = { name: model.modelId };
                if (model.label && model.label !== model.modelId) value.labelOverride = model.label;
                if (model.oneM) value.supports1m = true;
                return Object.keys(value).length === 1 ? model.modelId : value;
              }));
            }
          }
        }
        if (review.desktopHoldPreview) {
          return new Promise(resolve => review.desktopPreviewResolvers.push(() => resolve(preview)));
        }
        return preview;
      }
      if (command === "commit_claude_desktop_change") {
        review.desktopCalls.commit.push(clone(args));
        if (review.desktopCommitFailure) throw review.desktopCommitFailure;
        if (args.action === "applyRouter") {
          Object.assign(review.desktopStatus, { mode: "router", managementState: "managed", isManaged: true,
            staleReasons: [], conflicts: [], providerMissing: false, providerId: null, providerName: null,
            modelChoices: [], routeChoices: clone(args.request.routes),
            routeViews: args.request.routes.map(choice => {
              const target = review.desktopCandidates.routeProviders.find(entry => entry.providerId === choice.providerId);
              const candidate = target?.models.find(entry => entry.modelId === choice.modelId);
              return { ...choice, providerName: target?.name, label: choice.labelOverride ?? candidate?.label,
                oneM: choice.oneMMode === "auto", protocol: candidate?.protocol, available: true, reason: null };
            }), router: { boundPort: 43127, routeRevision: 2, gatewayTokenVersion: 1, listenerReady: true }, restartRequired: true,
            restartCapable: true, appRunning: false });
          review.desktopStatus.actions.find(entry => entry.action === "release").enabled = false;
          review.desktopStatus.actions.find(entry => entry.action === "release").disabledReason = "claudeDesktop.actionReasons.routerReleaseBlocked";
        }
        return clone(review.desktopApplied);
      }
      if (command === "cancel_claude_desktop_preview") { review.desktopCalls.cancelPreview.push(args.revision); return; }
      if (command === "probe_claude_desktop_provider") {
        review.desktopCalls.probe.push(clone(args));
        if (review.desktopProbeFailure) throw review.desktopProbeFailure;
        if (review.desktopHoldProbe) return new Promise(resolve => review.desktopProbeResolvers.push(() => resolve(clone(review.desktopProbe))));
        return clone(review.desktopProbe);
      }
      if (command === "delete_newapi_provider") {
        const state = review.getState();
        state.providers = state.providers.filter(provider => provider.id !== args.id);
        review.desktopStatus.managementState = "orphaned";
        review.desktopStatus.providerMissing = true;
        review.desktopCandidates.providers = review.desktopCandidates.providers.filter(provider => provider.providerId !== args.id);
        return { state: clone(state), applied: null };
      }
      if (command === "restart_claude_desktop") {
        review.desktopCalls.restart++;
        if (review.desktopRestartFailure) throw review.desktopRestartFailure;
        review.desktopStatus.restartRequired = false;
        review.desktopStatus.warnings = review.desktopStatus.warnings.filter(key => key !== "claudeDesktop.warnings.requiresRestart");
        review.desktopStatus.appRunning = true;
        return { launchMethod: "windowsExe", running: true, restartCapable: true };
      }
      review.desktopCalls.other.push(command);
      throw new Error("unexpected command in the Claude Desktop card suite: " + command);
    };
  }, { status: statusFixture, candidates: candidatesFixture, preview: previewFixture,
    applied: appliedFixture, probe: probeFixture });

  // api.claudeDesktop is the only api.* module the fixture does not stub, and a QueryObserver is
  // bound to the client it was created with: the card that mounted during reset() already failed
  // its status query, so route through another page to drop that observer before mounting the
  // card again - its first query then runs against the fake IPC.
  await reset("groups");
  await install();
  await page.evaluate(() => { window.review.mount("mcp"); });
  await page.evaluate(() => { window.review.mount("groups"); });
  await page.locator("#claude-desktop-title").waitFor();
  await page.waitForFunction(() => !document.querySelector('section[aria-labelledby="claude-desktop-title"] [role="alert"]'),
    null, { timeout: 20000 }).catch(async () => {
      throw new Error("the card never received its status: " + (await card().innerText()));
    });
  await actionButton(ZH.change).waitFor({ timeout: 20000 });
  assert.deepEqual(await page.evaluate(() => window.review.desktopCalls.other), [],
    "the card must only use its five IPC commands");

  // 1. Own labelled section, sibling of the untouched client-configs section.
  const structure = await page.evaluate(() => {
    const clients = document.querySelector('section[aria-labelledby="client-configs-title"]');
    const desktop = document.querySelector('section[aria-labelledby="claude-desktop-title"]');
    const heading = document.getElementById("claude-desktop-title");
    return {
      desktopSections: document.querySelectorAll('section[aria-labelledby="claude-desktop-title"]').length,
      clientsPresent: Boolean(clients),
      clientsHeading: document.getElementById("client-configs-title")?.textContent?.trim() ?? null,
      desktopFollowsClients: Boolean(clients && desktop) &&
        (clients.compareDocumentPosition(desktop) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0,
      nested: Boolean(clients && desktop) && (clients.contains(desktop) || desktop.contains(clients)),
      headingTag: heading?.tagName ?? null,
      headingText: heading?.textContent?.trim() ?? null,
      // The fixture only seeds client statuses for its "clients*" mounts, which do not render
      // <NewApiPage />; this mount still proves the sibling section mounts its own content.
      clientsGrid: Boolean(clients?.querySelector("div.grid")),
      clientsAlerts: clients ? clients.querySelectorAll('[role="alert"]').length : -1,
    };
  });
  assert.equal(structure.desktopSections, 1, "the card must be exactly one section");
  assert.equal(structure.clientsPresent, true, "the sibling client-configs section must still render");
  assert.equal(structure.clientsHeading, ZH.clientsTitle);
  assert.equal(structure.clientsGrid, true, "the client-configs card must still render its own content");
  assert.equal(structure.clientsAlerts, 0, "the client-configs card must stay error-free next to the Desktop card");
  assert.equal(structure.desktopFollowsClients, true, "the card must stay after the client-configs section");
  assert.equal(structure.nested, false, "the card must not be nested inside the client-configs section");
  assert.equal(structure.headingTag, "H2", "aria-labelledby must resolve to the section heading");
  assert.equal(structure.headingText, ZH.title);
  // A <section> with an accessible name is a region landmark: this uses Playwright's
  // accessible-name computation over aria-labelledby instead of a text lookup.
  const region = page.getByRole("region", { name: ZH.title, exact: true });
  assert.equal(await region.count(), 1, "the card must expose its title as the region accessible name");
  assert.equal(await region.getAttribute("aria-labelledby"), "claude-desktop-title");
  console.log("PASS Claude Desktop card renders as its own labelled section after the untouched client-configs section");

  // 2. Card content: badges, long path, long model id, warnings, action buttons.
  const content = await card().evaluate(node => {
    const text = selector => node.querySelector(selector)?.textContent?.trim() ?? null;
    const styles = selector => {
      const element = node.querySelector(selector);
      if (!element) return null;
      const style = getComputedStyle(element);
      return { wordBreak: style.wordBreak, overflowWrap: style.overflowWrap, fontFamily: style.fontFamily,
        scrollWidth: element.scrollWidth, clientWidth: element.clientWidth, className: element.className };
    };
    return {
      innerText: node.innerText,
      badgeRow: [...node.querySelectorAll("div.flex.flex-wrap.items-center.gap-2 > *")]
        .map(entry => entry.textContent.trim()).filter(Boolean),
      hint: text("[data-desktop-provider]"),
      models: text("p.break-words.font-mono.text-xs"),
      modelsStyle: styles("p.break-words.font-mono.text-xs"),
      path: text("p.break-all.font-mono"),
      pathStyle: styles("p.break-all.font-mono"),
      statuses: [...node.querySelectorAll('[role="status"]')].map(element => element.textContent.trim()),
      alerts: node.querySelectorAll('[role="alert"]').length,
      // The section title is a fold toggle (aria-expanded), not one of the card's actions.
      foldToggles: [...node.querySelectorAll("button[aria-expanded]")].map(button => button.innerText.trim()),
      buttons: [...node.querySelectorAll("button:not([aria-expanded])")].map(button => ({ text: button.innerText.trim(),
        disabled: button.disabled, title: button.getAttribute("title") })),
      overflow: node.scrollWidth - node.clientWidth,
    };
  });
  assert.equal(content.path, LONG_PATH, "the card must render the managed config path verbatim");
  assert.ok(content.pathStyle, "the config path line is missing");
  assert.equal(content.pathStyle.wordBreak, "break-all", "the config path must break long segments");
  assert.match(content.pathStyle.fontFamily, /mono/i, "the config path must stay monospace");
  assert.equal(content.pathStyle.scrollWidth <= content.pathStyle.clientWidth + 1, true, JSON.stringify(content.pathStyle));
  assert.equal(content.models, LONG_MODEL_ID + " · " + SHORT_MODEL_ID, "the card must list the managed models");
  assert.ok(content.modelsStyle, "the managed model line is missing");
  assert.equal(content.modelsStyle.overflowWrap, "break-word", "the model list must wrap long ids");
  assert.equal(content.modelsStyle.scrollWidth <= content.modelsStyle.clientWidth + 1, true, JSON.stringify(content.modelsStyle));
  assert.equal(content.hint, statusFixture.providerName, "the card must name the actual saved relay");
  assert.equal(content.alerts, 0);
  assert.deepEqual(content.statuses, [], "an unrelated disabled recovery action must not produce normal-state warnings");
  assert.doesNotMatch(content.innerText, /将 Claude Desktop 桌面客户端直连到中转|与 Claude Code 相互独立|没有未完成的事务|请完全退出/);
  const badgeTexts = content.badgeRow.join(" | ");
  for (const badge of [ZH.stateManaged, ZH.modeDirect, "v1.2.3", ZH.requiresRestart]) {
    assert.ok(badgeTexts.includes(badge), "missing card badge " + badge + " in " + badgeTexts);
  }
  assert.equal(await card().getByText(ZH.requiresRestart, { exact: true }).evaluate(element => getComputedStyle(element).animationName), "none",
    "the restart-required badge is static");
  await page.evaluate(() => document.documentElement.style.setProperty("--primary", "156 60% 40%"));
  await page.waitForFunction(() => [...document.querySelectorAll('[aria-labelledby="claude-desktop-title"] div')]
    .some(element => element.textContent.trim() === "已配置" && getComputedStyle(element).color === "rgb(41, 163, 114)"));
  assert.equal(await card().getByText(ZH.stateManaged, { exact: true }).evaluate(element => getComputedStyle(element).color), "rgb(41, 163, 114)",
    "the configured badge follows the current theme accent");
  await page.evaluate(() => document.documentElement.style.removeProperty("--primary"));
  assert.equal(await card().getByText("配置详情", { exact: true }).count(), 1);
  assert.doesNotMatch(content.innerText, /本地恢复备份: 3/, "backup details should stay collapsed by default");
  assert.doesNotMatch(content.innerText, /未消费的预览|AGENT_HARBOR_CLIENT_HOME/, "technical state belongs in details");
  await card().locator("details > summary").click();
  assert.match(await card().innerText(), /本地恢复备份: 3/);
  assert.match(await card().innerText(), /未消费的预览|AGENT_HARBOR_CLIENT_HOME/);
  await card().locator("details > summary").click();
  assert.deepEqual(content.foldToggles, [ZH.title], "the section title is the only fold toggle");
  assert.deepEqual(content.buttons.map(button => button.text),
    [ZH.change, ZH.reapply, ZH.restorePrevious, ZH.switchOfficial, ZH.release],
    "the card must expose its manage and confirmable action buttons");
  const byText = label => content.buttons.find(button => button.text === label);
  for (const enabled of [ZH.change, ZH.reapply, ZH.restorePrevious, ZH.release]) {
    assert.equal(byText(enabled).disabled, false, enabled + " must be enabled when the backend allows it");
  }
  assert.deepEqual({ disabled: byText(ZH.switchOfficial).disabled, title: byText(ZH.switchOfficial).title },
    { disabled: true, title: ZH.conflictReason });

  // 3. Widths and browser zoom: no sideways scrolling, every action still clickable.
  const matrix = [];
  for (const width of [1280, 1024, 768, 480]) {
    for (const zoom of [80, 100, 125, 150]) {
      await page.setViewportSize({ width, height: 900 });
      await page.evaluate(value => { document.documentElement.style.zoom = value + "%"; }, zoom);
      const metrics = await card().evaluate(node => {
        const root = document.documentElement;
        // CSS zoom scales getBoundingClientRect but not window.innerWidth: a layout that keeps its
        // content inside the scaled layout box therefore reports every visual edge inside the
        // window. Scroll natively (elementFromPoint and the rects share that visual space).
        const buttons = [...node.querySelectorAll("button")].map(button => {
          button.scrollIntoView({ block: "center" });
          const rect = button.getBoundingClientRect();
          const hit = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
          return { text: button.innerText.trim(), disabled: button.disabled, width: Math.round(rect.width),
            height: Math.round(rect.height), left: rect.left, right: rect.right,
            // Disabled buttons carry pointer-events: none, so they are deliberately not hit targets.
            hits: button.disabled || (Boolean(hit) && (button === hit || button.contains(hit))) };
        });
        const rects = [...node.querySelectorAll("*")].map(element => element.getBoundingClientRect());
        return { documentOverflow: root.scrollWidth - root.clientWidth,
          bodyOverflow: document.body.scrollWidth - document.body.clientWidth,
          sectionOverflow: node.scrollWidth - node.clientWidth,
          widestRight: Math.round(Math.max(...rects.map(rect => rect.right))),
          leftmostLeft: Math.round(Math.min(...rects.map(rect => rect.left))),
          layoutWidth: root.clientWidth, innerWidth: window.innerWidth, buttons };
      });
      matrix.push({ width, zoom, ...metrics });
      assert.ok(metrics.documentOverflow <= 1, width + "px @ " + zoom + "%: page overflows horizontally by " + metrics.documentOverflow);
      assert.ok(metrics.bodyOverflow <= 1, width + "px @ " + zoom + "%: body overflows horizontally by " + metrics.bodyOverflow);
      assert.equal(metrics.sectionOverflow <= 1, true, width + "px @ " + zoom + "%: the card itself overflows horizontally");
      assert.ok(metrics.leftmostLeft >= -1, width + "px @ " + zoom + "%: card content starts off-screen (" + metrics.leftmostLeft + ")");
      assert.ok(metrics.widestRight <= metrics.innerWidth + 1,
        width + "px @ " + zoom + "%: card content spills past the window (" + metrics.widestRight + " > " + metrics.innerWidth + ")");
      for (const button of metrics.buttons) {
        assert.ok(button.width > 0 && button.height > 0,
          width + "px @ " + zoom + "%: " + button.text + " collapsed to " + JSON.stringify(button));
        assert.ok(button.left >= -1 && button.right <= metrics.innerWidth + 1,
          width + "px @ " + zoom + "%: " + button.text + " is clipped horizontally: " + JSON.stringify(button));
        // A disabled button deliberately has pointer-events: none, so it can never be a hit
        // target; its geometry is still asserted above. Only enabled buttons must be hittable.
        if (!button.disabled) {
          assert.equal(button.hits, true,
            width + "px @ " + zoom + "%: " + button.text + " is covered or clipped at its centre");
        }
      }
      // The title (fold toggle) is hit-tested above but is not a manage action.
      assert.deepEqual(metrics.buttons.filter(button => !button.disabled && button.text !== ZH.title).map(button => button.text),
        [ZH.change, ZH.reapply, ZH.restorePrevious, ZH.release],
        width + "px @ " + zoom + "%: the set of clickable manage actions changed");
      // Playwright's own actionability check (visible, stable, receives pointer events at the
      // centre, enabled) on top of the page-side hit test above.
      for (const button of await card().getByRole("button").all()) {
        if (await button.isEnabled()) await button.click({ trial: true });
      }
      if (captureDir && ((width === 1280 && zoom === 100) || (width === 480 && zoom === 150))) {
        await card().evaluate(node => node.scrollIntoView({ block: "center" }));
        await page.screenshot({ path: path.join(captureDir, "claude-desktop-card-" + width + "-zoom" + zoom + ".png"), animations: "disabled" });
      }
    }
  }
  assert.equal(matrix.length, 16);
  await page.evaluate(() => { document.documentElement.style.removeProperty("zoom"); });
  console.log("PASS the Desktop card and its action buttons hold 1280/1024/768/480px at 80-150% zoom without sideways scrolling or clipping");

  await page.setViewportSize({ width: 1200, height: 700 });

  // 4. Keyboard: the configure button is reachable and activatable without a pointer.
  const change = actionButton(ZH.change);
  await change.focus();
  assert.equal(await change.evaluate(node => node === document.activeElement), true, "the configure button must be focusable");
  await page.keyboard.press("Enter");
  await dialog().waitFor();
  await dialog().getByRole("heading", { name: ZH.dialogTitle, exact: true }).waitFor();
  await dialog().getByRole("button", { name: /Relay Alpha/ }).waitFor();
  assert.equal(await page.evaluate(() => document.querySelector('[role="dialog"]').contains(document.activeElement)), true,
    "opening the dialog from the keyboard must move focus into it");
  assert.equal(await page.evaluate(() => window.review.desktopCalls.candidates.length), 1);
  console.log("PASS the configure button is focusable and opens the dialog from the keyboard");

  const dialogSurface = await dialog().evaluate(node => {
    const scrollArea = node.querySelector("[data-claude-desktop-scroll]");
    const fieldset = node.querySelector("fieldset");
    const buttonNamed = text => [...node.querySelectorAll("button")].find(button => button.innerText.includes(text));
    // The dialog clips its own content (overflow-hidden), so this is the harness-wide
    // "no horizontal overflow" contract; the long model rows are measured separately below.
    const box = scrollArea?.getBoundingClientRect();
    const limit = box ? box.right - (parseFloat(getComputedStyle(scrollArea).paddingRight) || 0) : null;
    return {
      hasScrollContainer: Boolean(scrollArea),
      dialogOverflow: node.scrollWidth - node.clientWidth,
      modelRows: scrollArea ? [...scrollArea.querySelectorAll("li")].map(row => {
        const label = row.querySelector("label");
        return { text: label?.textContent?.trim() ?? "", className: String(label?.className ?? ""),
          right: Math.round(row.getBoundingClientRect().right), limit: Math.round(limit),
          rowOverflow: row.scrollWidth - row.clientWidth,
          labelTruncated: label ? label.scrollWidth > label.clientWidth : null };
      }) : [],
      fieldsetDisabled: Boolean(fieldset?.disabled),
      providerButtons: [...node.querySelectorAll("button[data-desktop-provider-choice]")].map(button => ({ text: button.innerText.trim().split("\n")[0],
        pressed: button.getAttribute("aria-pressed"), disabled: button.disabled })),
      messagesCheckDisabled: buttonNamed("Messages")?.disabled ?? null,
      probeDisabled: buttonNamed("Bearer")?.disabled ?? null,
    };
  });
  // Radix animates the dialog in; measure only after fonts and two frames settled.
  await dialog().evaluate(async () => {
    await document.fonts.ready;
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  });
  assert.equal(dialogSurface.hasScrollContainer, true, "the dialog must expose its data-claude-desktop-scroll container");
  assert.ok(dialogSurface.dialogOverflow <= 1, "the dialog must not overflow horizontally: " + dialogSurface.dialogOverflow);

  assert.equal(dialogSurface.modelRows.length, 2,
    "only selectable models may render a row: " + JSON.stringify(dialogSurface.modelRows));
  for (const row of dialogSurface.modelRows) {
    assert.ok(row.right <= row.limit + 1, "a model row must stay inside the dialog: " + JSON.stringify(row));
    assert.equal(row.rowOverflow <= 1, true, "a model row must not scroll sideways: " + JSON.stringify(row));
  }
  const longRowFit = dialogSurface.modelRows.find(row => row.text === LONG_MODEL_ID);
  assert.ok(longRowFit, "the long model id must have its own row");
  assert.match(longRowFit.className, /truncate/, "a long model id must truncate instead of widening the row");
  assert.equal(longRowFit.labelTruncated, true, "the long model id must actually be truncated at this width");
  assert.equal(dialogSurface.fieldsetDisabled, false, "the fieldset must only lock while a request is in flight");
  assert.deepEqual(dialogSurface.providerButtons, [{ text: "Relay Alpha", pressed: "true", disabled: false },
    { text: "Relay Beta", pressed: "false", disabled: true }]);
  assert.equal(dialogSurface.messagesCheckDisabled, false, "an existing configuration must start with its selected models");
  assert.equal(dialogSurface.probeDisabled, false);
  assert.equal(await dialog().getByText(ZH.providerDisabled, { exact: true }).count(), 1,
    "an ineligible provider must explain why it cannot be picked");
  assert.equal(await dialog().getByText(ZH.noInferenceKey, { exact: true }).count(), 1,
    "an existing endpoint must not hide the credential exclusion reason");
  assert.equal(await dialog().getByRole("button", { name: ZH.preview, exact: true }).isEnabled(), true,
    "opening an existing configuration must retain its saved choices");
  assert.doesNotMatch(await dialog().innerText(), /相互独立|使用卡片上已保存的推理密钥|预览只显示受管改动/);

  // Model rows: checkbox id <-> label for pairing, then Tab reachability and Space toggling.
  const modelIds = [LONG_MODEL_ID, SHORT_MODEL_ID];
  for (const modelId of modelIds) {
    assert.equal(await modelCheckbox(modelId).getAttribute("aria-checked"), "true", "saved choices must be checked");
    const row = dialog().locator("li").filter({ has: modelCheckbox(modelId) });
    assert.equal(await row.getByRole("textbox").inputValue(), "", "automatic labels must remain unpinned");
    assert.equal(await row.getByRole("checkbox", { name: "1M", exact: true }).getAttribute("aria-checked"), "true",
      "saved Auto intent must survive opening");
    if (modelId === SHORT_MODEL_ID) {
      const legacyOneM = row.getByRole("checkbox", { name: "1M", exact: true });
      assert.equal(await legacyOneM.isEnabled(), true, "an existing Auto choice stays cancellable when capability is unavailable");
      await legacyOneM.click();
      assert.equal(await legacyOneM.getAttribute("aria-checked"), "false");
      assert.equal(await legacyOneM.isDisabled(), true, "after cancellation an unsupported choice cannot be newly enabled");
      assert.equal(await legacyOneM.locator("..").getAttribute("title"), null, "the 1M control has no explanatory tooltip");
    }
    await modelCheckbox(modelId).uncheck();
  }
  // The dialog also renders the rebaseline checkbox, so count the model checkboxes by id.
  assert.equal(await dialog().locator('[id^="desktop-model-"]').count(), modelIds.length, "only selectable models may render a row");
  for (const modelId of modelIds) {
    const pairing = await page.evaluate(id => {
      const checkbox = document.getElementById("desktop-model-" + id);
      const label = document.querySelector('label[for="desktop-model-' + id + '"]');
      return {
        checkboxes: document.querySelectorAll('[id="desktop-model-' + id + '"]').length,
        tag: checkbox?.tagName ?? null,
        role: checkbox?.getAttribute("role") ?? null,
        labelFor: label?.getAttribute("for") ?? null,
        labelText: label?.textContent?.trim() ?? null,
        labelControlIsCheckbox: Boolean(label && checkbox) && label.control === checkbox,
      };
    }, modelId);
    assert.equal(pairing.checkboxes, 1, "duplicate or missing checkbox id for " + modelId);
    assert.equal(pairing.tag, "BUTTON");
    assert.equal(pairing.role, "checkbox");
    assert.deepEqual({ for: pairing.labelFor, text: pairing.labelText, control: pairing.labelControlIsCheckbox },
      { for: "desktop-model-" + modelId, text: modelId, control: true },
      "label htmlFor must resolve to the checkbox for " + modelId);
    assert.equal(await page.getByRole("checkbox", { name: modelId, exact: true }).count(), 1,
      "the associated label must give the checkbox its accessible name: " + modelId);
  }
  assert.equal(await dialog().getByText("chat-only-model", { exact: true }).count(), 1, "excluded models must explain their reason");
  assert.match(await dialog().locator("[data-desktop-excluded-models]").innerText(), /协议不是 Anthropic Messages/);

  const tabWalk = [];
  await dialog().getByRole("button", { name: /Relay Alpha/ }).focus();
  for (let step = 0; step < 90; step++) {
    const focused = await focusedId();
    if (focused) tabWalk.push(focused);
    await page.keyboard.press("Tab");
  }
  const reachedModels = [...new Set(tabWalk.filter(id => id.startsWith("desktop-model-")))]
    .map(id => id.slice("desktop-model-".length));
  assert.deepEqual(reachedModels, modelIds, "every model checkbox must be reachable with Tab, in DOM order: " + tabWalk.join(" > "));
  for (const modelId of modelIds) {
    const checkbox = modelCheckbox(modelId);
    assert.equal(await tabTo("desktop-model-" + modelId), true, "checkbox not reachable with Tab: " + modelId);
    await page.keyboard.press("Space");
    assert.equal(await checkbox.getAttribute("aria-checked"), "true", "Space must select " + modelId);
    await page.keyboard.press("Space");
    assert.equal(await checkbox.getAttribute("aria-checked"), "false", "Space must deselect " + modelId);
  }
  assert.equal(await dialog().getByRole("button", { name: ZH.messagesCheck, exact: true }).isDisabled(), true);
  console.log("PASS every model checkbox is tabbable, labelled through htmlFor and toggled with Space");

  // Select both models, then click and keyboard-toggle the 1M checkbox on the long row.
  for (const modelId of modelIds) await modelCheckbox(modelId).check();
  assert.equal(await dialog().getByRole("button", { name: ZH.messagesCheck, exact: true }).isEnabled(), true);
  const longRow = dialog().locator("li").filter({ has: page.locator('[id="desktop-model-' + LONG_MODEL_ID + '"]') });
  const oneM = longRow.getByRole("checkbox", { name: "1M", exact: true });
  assert.equal(await oneM.count(), 1, "a selected model must expose the 1M checkbox");
  assert.equal(await longRow.getByRole("textbox", { name: ZH.labelPlaceholder, exact: true }).getAttribute("maxlength"), "64");
  assert.equal(await oneM.getAttribute("aria-checked"), "true", "supported new direct choices enable 1M by default");
  assert.equal(await longRow.getByRole("textbox").inputValue(), "", "new automatic labels are placeholders, not overrides");
  const shortRow = dialog().locator("li").filter({ has: modelCheckbox(SHORT_MODEL_ID) });
  assert.equal(await shortRow.getByRole("checkbox", { name: "1M", exact: true }).isDisabled(), true,
    "a new selection cannot offer an unsupported 1M variant");
  await oneM.click();
  assert.equal(await oneM.getAttribute("aria-checked"), "false", "a single click cancels the default 1M intent");
  await oneM.focus();
  await page.keyboard.press("Space");
  assert.equal(await oneM.getAttribute("aria-checked"), "true", "Space toggles the labelled 1M checkbox on");
  await oneM.uncheck();
  assert.equal(await dialog().getByRole("combobox").count(), 1, "only saved configurations may use a dropdown; installation and 1M stay one-click controls");
  assert.equal(await dialog().locator("#desktop-saved-config").count(), 1);
  assert.doesNotMatch(await dialog().innerText(), /协议由模型名称推断|未经上游验证/, "the protocol inference badge is omitted");
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "claude-desktop-dialog.png"), animations: "disabled" });

  // 5. Busy state, preview request shape and the preview file cards.
  await page.evaluate(() => { window.review.desktopHoldPreview = true; });
  await dialog().getByRole("button", { name: ZH.preview, exact: true }).click();
  await page.waitForFunction(() => window.review.desktopPreviewResolvers.length === 1);
  const fieldset = scroll().locator("fieldset");
  assert.equal(await fieldset.evaluate(node => node.disabled), true, "the fieldset must lock while previewing");
  for (const modelId of modelIds) {
    assert.equal(await modelCheckbox(modelId).evaluate(node => node.matches(":disabled")), true, "a busy dialog must lock " + modelId);
  }
  assert.equal(await dialog().getByRole("button", { name: ZH.cancel, exact: true }).isDisabled(), true);
  assert.equal(await dialog().locator('[role="status"]').filter({ hasText: ZH.busyHint }).count(), 1,
    "a busy dialog must say why its controls are locked");
  await page.evaluate(() => { window.review.desktopPreviewResolvers.shift()(); window.review.desktopHoldPreview = false; });
  await dialog().locator("[data-desktop-preview-files]").waitFor();
  assert.equal(await fieldset.evaluate(node => node.disabled), false, "the fieldset must unlock when the request settles");
  assert.deepEqual(await page.evaluate(() => window.review.desktopCalls.preview[0]), {
    action: "applyDirect",
    request: {
      providerId: "group-a",
      models: [
        // The long row was switched off through its checkbox above.
        { modelId: LONG_MODEL_ID, labelOverride: null, oneMMode: "off" },
        { modelId: SHORT_MODEL_ID, labelOverride: null, oneMMode: "off" },
      ],
      paths: { normalRoot: NORMAL_ROOT, thirdPartyRoot: THIRD_PARTY_ROOT },
      rebaseline: false,
    },
  });
  assert.equal(await dialog().locator("[data-desktop-preview-summary]").count(), 0, "preview must not duplicate provider and model summaries");
  const modelAfter = dialog().locator('[data-desktop-managed-change="inferenceModels"] [data-desktop-change-side="after"] pre');
  assert.deepEqual(JSON.parse(await modelAfter.innerText()), modelIds, "the file preview must show the actual selected IDs in inferenceModels after");
  assert.equal(await modelAfter.isVisible(), true, "file changes are directly visible without expanding details");
  const previewCards = await dialog().evaluate(node => {
    const files = node.querySelector("[data-desktop-preview-files]")?.innerText ?? "";
    return { files, primary: files,
      statuses: [...node.querySelectorAll('[role="status"]')].map(element => element.textContent.trim()),
      pathsFit: [...node.querySelectorAll("p.font-mono")].every(entry => entry.scrollWidth <= entry.clientWidth + 1),
      overflow: node.scrollWidth - node.clientWidth };
  });
  assert.ok(previewCards.primary.includes("常规配置"), "preview files must localize their roles");
  assert.ok(previewCards.primary.includes("更新"), JSON.stringify(previewCards.primary));
  assert.ok(previewCards.primary.includes(LONG_PATH), "the preview card must show the managed file path");
  assert.match(previewCards.primary, /deploymentMode[\s\S]*原配置[\s\S]*1p[\s\S]*新配置[\s\S]*3p/, "managed before/after values must remain available");
  assert.ok(previewCards.files.includes("第三方配置") && previewCards.files.includes("新建"));
  assert.equal(previewCards.statuses.includes(ZH.warningRequiresRestart), false, "preview must not repeat exit warnings");
  assert.equal(await dialog().getByText(ZH.externalProfiles + ": 1", { exact: true }).count(), 1,
    "the preview must report the preserved external profiles");
  assert.equal(previewCards.pathsFit, true, "a long preview path must not spill out of its card");
  assert.ok(previewCards.overflow <= 1, "the preview must not widen the dialog: " + previewCards.overflow);
  console.log("PASS the dialog preserves saved intent, defaults supported new direct 1M choices on, previews models and reveals localized file changes");

  // 6. Apply commits the previewed revision, and a failed action surfaces as role=alert.
  await dialog().getByRole("button", { name: ZH.apply, exact: true }).click();
  await page.waitForFunction(() => window.review.desktopCalls.commit.length === 1);
  const commit = await page.evaluate(() => window.review.desktopCalls.commit[0]);
  assert.equal(commit.action, "applyDirect");
  assert.equal(commit.revision, previewFixture.revision);
  assert.deepEqual(commit.request, (await page.evaluate(() => window.review.desktopCalls.preview[0])).request,
    "apply must resend the previewed selection");
  await dialog().waitFor({ state: "hidden" });

  await page.evaluate(() => { window.review.desktopPreviewFailure = "claudeDesktop.errors.noInferenceKey"; });
  await actionButton(ZH.reapply).click();
  const confirm = page.getByRole("alertdialog");
  await confirm.getByRole("heading", { name: ZH.reapplyTitle, exact: true }).waitFor();
  assert.equal(await confirm.getByText(ZH.reapplyDescription, { exact: true }).count(), 1);
  assert.equal(await confirm.getByText(ZH.restoreDescription, { exact: true }).count(), 0,
    "reapply must not promise restoration");
  await confirm.getByRole("button", { name: ZH.confirm, exact: true }).click();
  // The confirm dialog stays open on failure and owns the first role=alert; the card behind it is
  // aria-hidden until the dialog closes, so both alerts are checked in order.
  await confirm.getByRole("alert").waitFor();
  assert.equal(await confirm.getByRole("alert").textContent(), ZH.noInferenceKey,
    "a failed management action must render the mapped error as role=alert");
  assert.equal(await page.evaluate(() => window.review.desktopCalls.preview.length), 2);
  await confirm.getByRole("button", { name: ZH.cancel, exact: true }).click();
  await confirm.waitFor({ state: "detached" });
  await card().getByRole("alert").waitFor();
  assert.equal(await card().getByRole("alert").textContent(), ZH.noInferenceKey,
    "the card must keep showing the failed action after the confirm dialog closes");
  await page.evaluate(() => { window.review.desktopPreviewFailure = null; });
  assert.equal(await actionButton(ZH.change).isEnabled(), true);
  console.log("PASS applying re-uses the previewed revision and failed management actions stay recoverable with role=alert");

  // 7. Reapply consumes the original intent, including auto on a currently non-1M model.
  await actionButton(ZH.reapply).click();
  await confirm.getByRole("heading", { name: ZH.reapplyTitle, exact: true }).waitFor();
  await confirm.getByRole("button", { name: ZH.confirm, exact: true }).click();
  await confirm.waitFor({ state: "detached", timeout: 5000 }).catch(async () => {
    throw new Error("reapply remained open: " + await confirm.innerText() + " calls=" + JSON.stringify(await page.evaluate(() => window.review.desktopCalls)));
  });
  assert.deepEqual(await page.evaluate(() => window.review.desktopCalls.preview.at(-1).request.models), statusFixture.modelChoices,
    "a resolved oneM=false must not turn the user's auto selection into off");
  console.log("PASS reapply has its own confirmation and preserves automatic 1M and unpinned labels");

  // 8. A real save callback must invalidate Desktop state; balance-only updates are not used.
  const beforeSave = await page.evaluate(() => window.review.desktopCalls.status);
  await page.evaluate(() => {
    window.review.desktopStatus.managementState = "stale";
    window.review.desktopStatus.staleReasons = ["credential"];
  });
  const providerFace = page.locator('[data-relay-card-face][data-provider-id="group-a"]');
  await providerFace.getByRole("button", { name: "编辑", exact: true }).click();
  await dialog().locator("#newapi-provider-name").fill("Relay Alpha edited");
  await dialog().getByRole("button", { name: "把 model-group-a 设为默认模型", exact: true }).click();
  await dialog().getByRole("button", { name: "保存", exact: true }).click();
  await dialog().waitFor({ state: "hidden" });
  await card().getByText("需重新应用", { exact: true }).waitFor();
  assert.ok(await page.evaluate(() => window.review.desktopCalls.status) > beforeSave,
    "saving the original provider must reload Desktop status without navigating away");
  assert.match(await card().innerText(), /推理密钥已变化/);
  assert.doesNotMatch(await card().innerText(), /credential/);
  console.log("PASS saving a provider refreshes Desktop stale status and translates the specific reason");

  // 9. Reports must disappear on provider/model/path changes, and a refreshed candidate invalidates a pending probe.
  await page.evaluate(() => {
    const review = window.review;
    const second = structuredClone(review.desktopCandidates.providers[0]);
    second.providerId = "group-c";
    second.name = "Relay Gamma";
    second.endpoint = { profileBaseUrl: "https://gamma.invalid", messagesUrl: "https://gamma.invalid/v1/messages",
      modelsUrl: "https://gamma.invalid/v1/models", plaintext: false };
    review.desktopCandidates.providers.push(second);
    review.desktopProbe.messages = { url: "https://relay.invalid/v1/messages", model: "claude-sonnet-4-5", status: "messagesAccessible",
      httpStatus: 200, responseModel: "claude-sonnet-4-5", stopReason: "end_turn", checkedAt: 1_790_000_000, note: null };
  });
  await actionButton(ZH.change).click();
  await dialog().getByRole("button", { name: /Relay Gamma/ }).waitFor();
  await modelCheckbox(LONG_MODEL_ID).uncheck();
  await modelCheckbox(SHORT_MODEL_ID).check();
  const success = dialog().getByText("Messages 端点接受裸 Bearer，并返回了合法的 Messages 响应", { exact: true });
  await dialog().getByRole("button", { name: ZH.messagesCheck, exact: true }).click();
  await success.waitFor();
  await page.evaluate(() => { window.review.desktopProbeFailure = "claudeDesktop.errors.noInferenceKey"; });
  await dialog().getByRole("button", { name: ZH.messagesCheck, exact: true }).click();
  await dialog().getByRole("alert").filter({ hasText: ZH.noInferenceKey }).waitFor();
  assert.equal(await success.count(), 0, "a failed recheck cannot retain an older green Messages report");
  await page.evaluate(() => { window.review.desktopProbeFailure = null; });
  await dialog().getByRole("button", { name: ZH.messagesCheck, exact: true }).click();
  await success.waitFor();
  await modelCheckbox(LONG_MODEL_ID).check();
  assert.equal(await success.count(), 0, "changing the selected model list must invalidate Messages evidence");
  await dialog().getByRole("button", { name: ZH.messagesCheck, exact: true }).click();
  await success.waitFor();
  await dialog().getByRole("button", { name: /Relay Gamma/ }).click();
  assert.equal(await success.count(), 0, "a new provider cannot retain another provider's green Messages report");
  assert.equal(await dialog().getByText("https://relay.invalid/v1/messages", { exact: true }).count(), 0,
    "the old endpoint must disappear with its evidence");
  await modelCheckbox(SHORT_MODEL_ID).check();
  await page.evaluate(() => { window.review.desktopHoldProbe = true; });
  await dialog().getByRole("button", { name: ZH.messagesCheck, exact: true }).click();
  await page.waitForFunction(() => window.review.desktopProbeResolvers.length === 1);
  // Even an identical candidate payload can hide a changed credential: dataUpdatedAt must invalidate it.
  await page.evaluate(() => window.review.providersClient.invalidateQueries({ queryKey: ["claude-desktop", "candidates"] }));
  await page.evaluate(() => { window.review.desktopProbeResolvers.shift()(); window.review.desktopHoldProbe = false; });
  await dialog().getByRole("button", { name: ZH.cancel, exact: true }).waitFor({ state: "visible" });
  await page.waitForFunction(() => !document.querySelector('[data-claude-desktop-scroll] fieldset')?.disabled);
  assert.equal(await success.count(), 0, "a late result from before a candidate/credential refresh must be ignored");
  await dialog().getByRole("button", { name: ZH.cancel, exact: true }).click();
  await dialog().waitFor({ state: "hidden" });
  console.log("PASS model/provider changes clear evidence and candidate refresh ignores an in-flight old probe");

  // 10. First discovery with no default still exposes all installations and requires an explicit choice.
  await page.evaluate(() => {
    const candidates = window.review.desktopCandidates;
    candidates.paths = null;
    candidates.selectedPathId = null;
    candidates.pathCandidates.push({ id: "path-beta", normalRoot: "C:/synthetic/Claude-Beta", thirdPartyRoot: "C:/synthetic/Claude-Beta-3p",
      normalExists: true, thirdPartyExists: false, hasInstallEvidence: true });
    window.review.mount("mcp");
  });
  await page.evaluate(() => window.review.mount("groups"));
  await actionButton(ZH.change).waitFor();
  await actionButton(ZH.change).click();
  await dialog().locator("#desktop-path").waitFor();
  await modelCheckbox(SHORT_MODEL_ID).check();
  assert.equal(await dialog().getByRole("button", { name: ZH.preview, exact: true }).isDisabled(), true);
  assert.equal(await dialog().getByRole("button", { name: ZH.messagesCheck, exact: true }).isDisabled(), true);
  assert.equal(await dialog().getByText("请先选择 Claude Desktop 的配置位置。", { exact: true }).count(), 2,
    "path selection and disabled-action explanation must both identify the missing choice");
  const pathSelect = dialog().locator("#desktop-path");
  await pathSelect.click();
  await page.getByRole("option", { name: "C:/synthetic/Claude-Beta", exact: true }).click();
  assert.equal(await dialog().getByRole("button", { name: ZH.preview, exact: true }).isEnabled(), true);
  await dialog().getByRole("button", { name: ZH.preview, exact: true }).click();
  await dialog().locator("[data-desktop-preview-files]").waitFor();
  assert.deepEqual(await page.evaluate(() => window.review.desktopCalls.preview.at(-1).request.paths),
    { normalRoot: "C:/synthetic/Claude-Beta", thirdPartyRoot: "C:/synthetic/Claude-Beta-3p" });
  await dialog().getByRole("button", { name: ZH.cancel, exact: true }).click();
  console.log("PASS multiple installations without a default remain selectable and bind the selected roots to preview");

  // 11. Catalog metadata refresh and provider deletion must invalidate the same projection.
  const beforeCatalog = await page.evaluate(() => window.review.desktopCalls.status);
  await page.evaluate(() => { window.review.desktopStatus.staleReasons = ["context:claude-sonnet-4-5"]; });
  const capabilityPanel = page.getByRole("region", { name: "模型能力目录与匹配预览" });
  await capabilityPanel.getByRole("button", { name: "模型能力目录与匹配预览", exact: true }).click();
  await capabilityPanel.getByRole("button", { name: "刷新目录", exact: true }).click();
  await card().getByText(/模型上下文能力已变化/).waitFor();
  assert.ok(await page.evaluate(() => window.review.desktopCalls.status) > beforeCatalog,
    "catalog refresh must reload the Desktop projection");
  const beforeDelete = await page.evaluate(() => window.review.desktopCalls.status);
  await page.locator('[data-relay-card-face][data-provider-id="group-a"]').getByRole("button", { name: "删除", exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: ZH.confirm, exact: true }).click();
  await card().getByText("供应商已删除", { exact: true }).waitFor();
  assert.ok(await page.evaluate(() => window.review.desktopCalls.status) > beforeDelete,
    "provider deletion must reload status before the user navigates away");
  console.log("PASS catalog refresh and provider deletion refresh Desktop capability and orphaned status");

  // 12. Restoring/switching may release management while still requiring a restart for the written files.
  await page.evaluate(async () => {
    Object.assign(window.review.desktopStatus, {
      managementState: "unmanaged", isManaged: false, mode: "official", providerId: null,
      providerName: null, providerMissing: false, models: [], modelChoices: [], staleReasons: [], warnings: [],
      restartRequired: true, restartCapable: true, appRunning: false, launchMethod: "windowsExe",
    });
    await window.review.providersClient.invalidateQueries({ queryKey: ["claude-desktop", "status"] });
  });
  await card().getByText("未配置", { exact: true }).waitFor();
  assert.equal(await actionButton(ZH.restart).isEnabled(), true,
    "a restored unmanaged configuration must still expose its required restart");
  await actionButton(ZH.restart).click();
  await page.waitForFunction(() => window.review.desktopCalls.restart === 1);
  await card().getByText(ZH.requiresRestart, { exact: true }).waitFor({ state: "hidden" });
  assert.equal(await card().getByText(ZH.requiresRestart, { exact: true }).count(), 0,
    "a successful restart must refresh status and remove the pending badge");
  await page.evaluate(async () => {
    window.review.desktopStatus.restartRequired = false;
    window.review.desktopStatus.appRunning = false;
    await window.review.providersClient.invalidateQueries({ queryKey: ["claude-desktop", "status"] });
  });
  await actionButton(ZH.restart).waitFor({ state: "hidden" });
  assert.equal(await actionButton(ZH.restart).count(), 0,
    "releasing management without pending file changes cannot invent a restart requirement");
  console.log("PASS an unmanaged restored configuration exposes its required restart and a clean release does not");

  // 13. Opus 4.7 appears as the new preview, independently of the old profile and the running app.
  await page.evaluate(() => {
    const review = window.review;
    review.desktopCandidates.selectedPathId = "path-normal";
    review.desktopCandidates.providers.push({
      providerId: "group-opus", name: "Opus Relay", enabled: true, hasInferenceKey: true, eligible: true, reason: null,
      endpoint: { profileBaseUrl: "https://opus.invalid", messagesUrl: "https://opus.invalid/v1/messages",
        modelsUrl: "https://opus.invalid/v1/models", plaintext: false },
      models: [{ modelId: "claude-opus-4-7", label: "Opus 4.7", displayName: "Opus 4.7", protocol: "anthropicMessages",
        protocolSource: "catalog", protocolVerified: true, included: true, supports1m: true, contextWindow: 1_000_000,
        contextSource: "catalog", selectable: true, excludeReason: null }],
    });
    review.desktopStatus.appRunning = true;
    review.desktopPreview.files = [{ role: "profile", path: "C:/synthetic/Claude-3p/configLibrary/profile.json", kind: "update",
      managedChanges: [{ path: "inferenceModels", before: '["claude-opus-4-6"]', after: '["claude-opus-4-7"]' }], note: null }];
  });
  await actionButton("配置").click();
  await dialog().getByRole("button", { name: /Opus Relay/ }).click();
  await modelCheckbox("claude-opus-4-7").check();
  const opusRow = dialog().locator("li").filter({ has: modelCheckbox("claude-opus-4-7") });
  assert.equal(await opusRow.getByRole("textbox").inputValue(), "");
  assert.equal(await opusRow.getByRole("textbox").getAttribute("placeholder"), "Opus 4.7");
  const opusOneM = opusRow.getByRole("checkbox", { name: "1M", exact: true });
  assert.equal(await opusOneM.getAttribute("aria-checked"), "true", "a supported new direct model starts with 1M checked");
  await opusOneM.uncheck();
  await dialog().getByRole("button", { name: ZH.preview, exact: true }).click();
  await dialog().locator("[data-desktop-preview-files]").waitFor();
  const opusPreview = dialog().locator("[data-desktop-preview-files]");
  const opusAfter = opusPreview.locator('[data-desktop-managed-change="inferenceModels"] [data-desktop-change-side="after"] pre');
  assert.deepEqual(JSON.parse(await opusAfter.innerText()), [{ name: "claude-opus-4-7", labelOverride: "Opus 4.7" }]);
  assert.doesNotMatch(await opusAfter.innerText(), /claude-opus-4-6/, "the old profile must not stand in for the new selection");
  assert.doesNotMatch(await opusAfter.innerText(), /supports1m/, "ordinary selection should create one Desktop menu option");
  assert.deepEqual(await page.evaluate(() => window.review.desktopCalls.preview.at(-1).request.models),
    [{ modelId: "claude-opus-4-7", labelOverride: null, oneMMode: "off" }]);
  await opusOneM.click();
  assert.equal(await opusPreview.count(), 0, "changing the 1M intent must invalidate its preview");
  await dialog().getByRole("button", { name: ZH.preview, exact: true }).click();
  await opusAfter.waitFor();
  assert.deepEqual(JSON.parse(await opusAfter.innerText()), [{ name: "claude-opus-4-7", labelOverride: "Opus 4.7", supports1m: true }],
    "the explicit checkbox enables the additional 1M variant in the actual new profile");
  assert.deepEqual(await page.evaluate(() => window.review.desktopCalls.preview.at(-1).request.models),
    [{ modelId: "claude-opus-4-7", labelOverride: null, oneMMode: "auto" }]);
  if (captureDir) {
    for (const width of [1200, 480]) {
      await page.setViewportSize({ width, height: 900 });
      await opusPreview.scrollIntoViewIfNeeded();
      const fit = await dialog().evaluate(node => ({ dialog: node.scrollWidth - node.clientWidth,
        files: node.querySelector("[data-desktop-preview-files]").scrollWidth - node.querySelector("[data-desktop-preview-files]").clientWidth }));
      assert.ok(fit.dialog <= 1 && fit.files <= 1, "file-only preview must fit at " + width + "px: " + JSON.stringify(fit));
      await page.screenshot({ path: path.join(captureDir, "claude-desktop-opus-file-preview-" + width + ".png"), animations: "disabled" });
    }
    await page.setViewportSize({ width: 1200, height: 700 });
  }
  await page.evaluate(() => { window.review.desktopCommitFailure = "claudeDesktop.errors.desktopRunning"; });
  await dialog().getByRole("button", { name: ZH.apply, exact: true }).click();
  await dialog().getByRole("alert").filter({ hasText: "请完全退出 Claude Desktop" }).waitFor();
  assert.equal(await dialog().getByRole("button", { name: ZH.preview, exact: true }).isEnabled(), true,
    "a failed commit consumes its ticket and must return to preview");
  assert.equal(await modelCheckbox("claude-opus-4-7").getAttribute("aria-checked"), "true", "failure must retain the selected model");
  assert.equal(await opusOneM.getAttribute("aria-checked"), "true", "failure must retain the 1M intent");
  await dialog().getByRole("button", { name: ZH.cancel, exact: true }).click();
  console.log("PASS a running Desktop allows Opus 4.7 preview, 1M can be unchecked, and failed apply retains input with a fresh-preview action");

  // 14. Saved selections that disappeared or became excluded remain visible and removable.
  await page.evaluate(async () => {
    const review = window.review;
    review.desktopCommitFailure = null;
    const provider = review.desktopCandidates.providers.find(entry => entry.providerId === "group-opus");
    provider.models.push({ ...provider.models[0], modelId: "claude-sonnet-4-6", label: "Sonnet 4.6",
      selectable: false, included: false, excludeReason: "model_not_included" });
    Object.assign(review.desktopStatus, {
      managementState: "stale", isManaged: true, mode: "direct", providerId: "group-opus", providerName: "Opus Relay",
      models: [{ modelId: "claude-opus-4-6", label: "My old Opus", oneM: true },
        { modelId: "claude-sonnet-4-6", label: "Sonnet 4.6", oneM: false }],
      modelChoices: [{ modelId: "claude-opus-4-6", labelOverride: "My old Opus", oneMMode: "auto" },
        { modelId: "claude-sonnet-4-6", labelOverride: null, oneMMode: "off" }],
    });
    await review.providersClient.invalidateQueries({ queryKey: ["claude-desktop", "status"] });
  });
  await actionButton(ZH.change).click();
  const unavailable = dialog().locator("[data-desktop-unavailable-choices]");
  await unavailable.getByRole("button", { name: "移除 claude-opus-4-6", exact: true }).waitFor();
  assert.match(await unavailable.innerText(), /claude-opus-4-6[\s\S]*My old Opus[\s\S]*已不在卡片中[\s\S]*添加/,
    "removed selections must expose their preserved label, 1M choice and specific reason");
  assert.match(await unavailable.innerText(), /claude-sonnet-4-6[\s\S]*该模型在卡片中被排除[\s\S]*不添加/,
    "an excluded saved selection must remain removable instead of being submitted invisibly");
  assert.equal(await dialog().locator("[data-desktop-excluded-models]").count(), 0, "a selected unavailable model must not be listed twice");
  assert.equal(await dialog().getByRole("button", { name: ZH.preview, exact: true }).isDisabled(), true,
    "unavailable selections must be resolved before preview");
  if (captureDir) {
    await unavailable.scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(captureDir, "claude-desktop-unavailable-selections.png"), animations: "disabled" });
  }
  await unavailable.getByRole("button", { name: "移除 claude-opus-4-6", exact: true }).click();
  await unavailable.getByRole("button", { name: "移除 claude-sonnet-4-6", exact: true }).click();
  assert.equal(await unavailable.count(), 0);
  await modelCheckbox("claude-opus-4-7").check();
  await dialog().getByRole("button", { name: ZH.preview, exact: true }).click();
  await dialog().locator("[data-desktop-preview-files]").waitFor();
  assert.deepEqual(await page.evaluate(() => window.review.desktopCalls.preview.at(-1).request.models),
    [{ modelId: "claude-opus-4-7", labelOverride: null, oneMMode: "auto" }],
    "after explicitly removing obsolete choices, only the new selected model is sent");
  await dialog().getByRole("button", { name: ZH.cancel, exact: true }).click();
  console.log("PASS removed and excluded saved models remain visible with their original intent and can be replaced explicitly");

  // R-A02/03/04/10/19/22: map roles explicitly, retaining existing cards and one profile preview.
  await page.evaluate(() => {
    const review = window.review;
    const candidate = (providerId, name, models, eligible = true) => ({ providerId, name, models, enabled: eligible,
      hasInferenceKey: true, eligible, reason: eligible ? null : "provider_disabled", endpoint: null });
    const model = (modelId, protocol, supports1m = false, selectable = true) => ({ modelId, label: modelId,
      displayName: modelId, protocol, protocolSource: "catalog", protocolVerified: true, included: selectable,
      supports1m, contextWindow: supports1m ? 1_000_000 : 200_000, contextSource: "catalog", selectable,
      excludeReason: selectable ? null : "model_not_included" });
    review.desktopCandidates.routeProviders = [
      candidate("route-chat", "Chat Relay", [model("deepseek-chat", "chatCompletions", true), model("small-chat", "chatCompletions"), model("excluded-model", "chatCompletions", false, false)]),
      candidate("route-messages", "Messages Relay", [model("claude-opus-4-7", "anthropicMessages", true), model("small-messages", "anthropicMessages")]),
      candidate("route-responses", "Responses Relay", [model("gpt-5", "responses", true), model("small-responses", "responses")]),
      candidate("route-disabled", "Disabled Relay", [model("disabled-model", "anthropicMessages")], false),
    ];
    review.desktopCandidates.routeRoles = [
      { role: "sonnet", roleAlias: "claude-sonnet-4-6" }, { role: "opus", roleAlias: "claude-opus-4-7" },
      { role: "haiku", roleAlias: "claude-haiku-4-5" },
    ];
    review.desktopStatus.routeChoices = [];
    review.desktopCandidates.pathCandidates = review.desktopCandidates.pathCandidates.slice(0, 1);
  });
  const stateBeforeRoutes = await page.evaluate(() => structuredClone(window.review.getState()));
  await actionButton(ZH.change).click();
  await dialog().getByRole("button", { name: "路由", exact: true }).click();
  const roleRow = role => dialog().locator(`[data-desktop-route-role="${role}"]`);
  const pick = async (role, field, option) => {
    await roleRow(role).getByRole("combobox", { name: `${role} ${field === "provider" ? "中转卡片" : "模型"}`, exact: true }).click();
    await page.getByRole("option", { name: option, exact: true }).click();
  };
  assert.equal(await dialog().getByText("未配置", { exact: true }).count(), 6,
    "all roles remain visibly empty until the user assigns them");
  assert.equal(await dialog().getByRole("button", { name: ZH.preview, exact: true }).isDisabled(), true);
  await roleRow("sonnet").getByRole("combobox", { name: "sonnet 中转卡片", exact: true }).click();
  assert.equal(await page.getByRole("option", { name: "Disabled Relay · 未启用", exact: true }).getAttribute("aria-disabled"), "true");
  await page.getByRole("option", { name: "Chat Relay", exact: true }).click();
  await roleRow("sonnet").getByRole("combobox", { name: "sonnet 模型", exact: true }).click();
  assert.equal(await page.getByRole("option", { name: "excluded-model · 不可用", exact: true }).getAttribute("aria-disabled"), "true");
  await page.getByRole("option", { name: "deepseek-chat", exact: true }).click();
  const sonnetRouteOneM = roleRow("sonnet").getByRole("checkbox", { name: "1M", exact: true });
  assert.equal(await sonnetRouteOneM.isEnabled(), true, "a capable Chat target can opt into 1M");
  assert.equal(await sonnetRouteOneM.getAttribute("aria-checked"), "false", "Chat 1M stays off until explicitly selected");
  await sonnetRouteOneM.click();
  assert.equal(await sonnetRouteOneM.getAttribute("aria-checked"), "true");
  await sonnetRouteOneM.click();
  await pick("sonnet", "model", "small-chat");
  assert.equal(await sonnetRouteOneM.isDisabled(), true, "Chat with insufficient context cannot offer 1M");
  await pick("sonnet", "model", "deepseek-chat");
  assert.match(await dialog().locator("[data-desktop-missing-roles]").textContent(), /haiku/,
    "partial routing warns about subagent roles without guessing their targets");
  await pick("opus", "provider", "Responses Relay");
  await pick("opus", "model", "gpt-5");
  const responsesRouteOneM = roleRow("opus").getByRole("checkbox", { name: "1M", exact: true });
  assert.equal(await responsesRouteOneM.isEnabled(), true, "a capable Responses target can opt into 1M");
  assert.equal(await responsesRouteOneM.getAttribute("aria-checked"), "false", "Responses 1M defaults off");
  await responsesRouteOneM.click();
  assert.equal(await responsesRouteOneM.getAttribute("aria-checked"), "true");
  await pick("opus", "model", "small-responses");
  assert.equal(await responsesRouteOneM.isDisabled(), true, "Responses with insufficient context cannot offer 1M");
  await pick("opus", "provider", "Messages Relay");
  await pick("opus", "model", "small-messages");
  assert.equal(await roleRow("opus").getByRole("checkbox", { name: "1M", exact: true }).isDisabled(), true,
    "Messages with insufficient context cannot offer 1M");
  await pick("opus", "model", "claude-opus-4-7");
  const opusRouteOneM = roleRow("opus").getByRole("checkbox", { name: "1M", exact: true });
  assert.equal(await opusRouteOneM.isEnabled(), true, "Messages uses the same effective context capability");
  assert.equal(await opusRouteOneM.getAttribute("aria-checked"), "false");
  assert.equal(await tabTo("desktop-route-opus-onem"), true, "role 1M toggle remains keyboard reachable");
  await page.keyboard.press("Space");
  await roleRow("sonnet").getByRole("textbox", { name: "sonnet 显示名称", exact: true }).fill("我的 Chat");
  await dialog().getByRole("checkbox", { name: "更新本地网关密钥", exact: true }).check();
  await dialog().getByRole("button", { name: ZH.preview, exact: true }).click();
  const routeFiles = dialog().locator("[data-desktop-preview-files]");
  await routeFiles.waitFor();
  const routeRequest = await page.evaluate(() => window.review.desktopCalls.preview.at(-1));
  assert.deepEqual(routeRequest, { action: "applyRouter", request: { routes: [
    { role: "sonnet", roleAlias: "claude-sonnet-4-6", providerId: "route-chat", modelId: "deepseek-chat", labelOverride: "我的 Chat", oneMMode: "off" },
    { role: "opus", roleAlias: "claude-opus-4-7", providerId: "route-messages", modelId: "claude-opus-4-7", labelOverride: null, oneMMode: "auto" },
  ], rotateGatewayToken: true, paths: { normalRoot: NORMAL_ROOT, thirdPartyRoot: THIRD_PARTY_ROOT }, rebaseline: false } });
  const roleProfile = JSON.parse(await routeFiles.locator('[data-desktop-managed-change="inferenceModels"] [data-desktop-change-side="after"] pre').innerText());
  assert.deepEqual(roleProfile, [{ name: "claude-sonnet-4-6", labelOverride: "我的 Chat" },
    { name: "claude-opus-4-7", labelOverride: "Messages Relay · claude-opus-4-7", supports1m: true }]);
  assert.equal(roleProfile.some(model => model.name === "deepseek-chat"), false);
  assert.equal(await dialog().locator("[data-desktop-preview-models]").count(), 0);
  assert.equal(await dialog().getByRole("button", { name: ZH.messagesCheck, exact: true }).count(), 0,
    "routing previews must not offer a direct-only upstream probe");
  assert.doesNotMatch(JSON.stringify(routeRequest), /apiKey|accessToken|gatewayToken"/);
  assert.deepEqual(await page.evaluate(() => window.review.getState()), stateBeforeRoutes,
    "choosing role targets cannot enable cards or change the global router/default model");
  console.log("PASS routing explicitly maps saved cards with empty roles, safe aliases, capability-based opt-in 1M across three protocols and no provider mutation");

  await page.evaluate(async () => {
    window.review.desktopCandidates.routeProviders.push({ providerId: "unrelated", name: "Unrelated", models: [], enabled: false, hasInferenceKey: false, eligible: false, reason: "provider_disabled", endpoint: null });
    await window.review.providersClient.invalidateQueries({ queryKey: ["claude-desktop", "candidates"] });
  });
  assert.equal(await routeFiles.count(), 1, "unrelated candidate updates must retain the current route file preview");
  await page.evaluate(async () => {
    window.review.desktopCandidates.routeProviders.find(provider => provider.providerId === "route-messages").models[0].label = "Opus updated";
    await window.review.providersClient.invalidateQueries({ queryKey: ["claude-desktop", "candidates"] });
  });
  await routeFiles.waitFor({ state: "hidden" });
  await page.waitForFunction(() => window.review.desktopCalls.cancelPreview.length > 0);
  await dialog().getByRole("button", { name: ZH.preview, exact: true }).click();
  await routeFiles.waitFor();
  {
    for (const theme of ["light", "dark"]) {
      await page.evaluate(value => document.documentElement.classList.toggle("dark", value === "dark"), theme);
      for (const width of [1200, 390]) {
        await page.setViewportSize({ width, height: 900 });
        await dialog().locator("[data-desktop-route-mapping]").scrollIntoViewIfNeeded();
        const overflow = await dialog().evaluate(node => ({ outer: node.scrollWidth - node.clientWidth,
          mapping: node.querySelector("[data-desktop-route-mapping]").scrollWidth - node.querySelector("[data-desktop-route-mapping]").clientWidth }));
        assert.ok(overflow.outer <= 1 && overflow.mapping <= 1, "routing controls overflow: " + JSON.stringify({ width, theme, overflow }));
        if (captureDir) await page.screenshot({ path: path.join(captureDir, `claude-desktop-routing-${width}-${theme}.png`), animations: "disabled" });
      }
    }
    await page.setViewportSize({ width: 1200, height: 700 });
    await page.evaluate(() => document.documentElement.classList.remove("dark"));
  }
  await page.evaluate(() => { window.review.desktopCommitFailure = "claudeDesktop.errors.previewExpired"; });
  await dialog().getByRole("button", { name: ZH.apply, exact: true }).click();
  await dialog().getByRole("alert").filter({ hasText: "该预览已过期" }).waitFor();
  assert.equal(await roleRow("sonnet").getByRole("textbox").inputValue(), "我的 Chat");
  assert.equal(await opusRouteOneM.getAttribute("aria-checked"), "true");
  await page.evaluate(() => { window.review.desktopCommitFailure = null; });
  await dialog().getByRole("button", { name: ZH.preview, exact: true }).click();
  await routeFiles.waitFor();
  await dialog().getByRole("button", { name: ZH.apply, exact: true }).click();
  await dialog().waitFor({ state: "hidden" });
  await card().getByText("AgentHarbor 路由", { exact: true }).waitFor();
  assert.equal(await card().getByText("网关未就绪", { exact: true }).count(), 0,
    "a ready listener must not add a normal-state warning");
  await page.evaluate(async () => {
    window.review.desktopStatus.router.listenerReady = false;
    await window.review.providersClient.invalidateQueries({ queryKey: ["claude-desktop", "status"] });
  });
  await card().getByText("网关未就绪", { exact: true }).waitFor();
  assert.equal(await card().getByText("已配置", { exact: true }).count(), 1,
    "listener availability is independent of the saved configuration state");
  assert.equal(await card().getByText("需重新应用", { exact: true }).count(), 0,
    "a listener outage alone must not claim the profile needs reapplying");
  await page.evaluate(async () => {
    window.review.desktopStatus.router.listenerReady = true;
    await window.review.providersClient.invalidateQueries({ queryKey: ["claude-desktop", "status"] });
  });
  await card().getByText("网关未就绪", { exact: true }).waitFor({ state: "hidden" });
  assert.match(await card().locator("[data-desktop-route-targets]").innerText(), /Chat Relay · deepseek-chat[\s\S]*Messages Relay · claude-opus-4-7/);
  assert.equal(await actionButton(ZH.release).isDisabled(), true, "localhost routing must restore before releasing management");
  await actionButton(ZH.reapply).click();
  await confirm.getByRole("button", { name: ZH.confirm, exact: true }).click();
  await confirm.waitFor({ state: "hidden" });
  assert.deepEqual(await page.evaluate(() => window.review.desktopCalls.preview.at(-1).request.routes), routeRequest.request.routes,
    "router reapply preserves role choices even without a single direct provider");
  await page.evaluate(() => { window.review.desktopRestartFailure = "claudeDesktop.errors.processStopTimeout"; });
  await actionButton(ZH.restart).click();
  await card().getByRole("alert").filter({ hasText: "无法结束 Claude Desktop" }).waitFor();
  assert.equal(await actionButton(ZH.restart).isEnabled(), true);
  assert.equal(await card().getByText(ZH.requiresRestart, { exact: true }).count(), 1);
  await page.evaluate(() => { window.review.desktopRestartFailure = null; });
  await actionButton(ZH.restart).click();
  await card().getByText(ZH.requiresRestart, { exact: true }).waitFor({ state: "hidden" });
  console.log("PASS routing previews release temporary leases, retain input after expired tickets, show actual targets and retry restart failures");

  await actionButton(ZH.change).click();
  assert.equal(await dialog().getByRole("button", { name: "路由", exact: true }).getAttribute("aria-pressed"), "true");
  await page.evaluate(async () => {
    const model = window.review.desktopCandidates.routeProviders.find(provider => provider.providerId === "route-messages").models[0];
    model.supports1m = false;
    model.contextWindow = 200_000;
    await window.review.providersClient.invalidateQueries({ queryKey: ["claude-desktop", "candidates"] });
  });
  const savedOneM = roleRow("opus").getByRole("checkbox", { name: "1M", exact: true });
  await roleRow("opus").getByText("请关闭额外 1M 选项或选择支持 1M 的模型。", { exact: true }).waitFor();
  assert.equal(await savedOneM.getAttribute("aria-checked"), "true", "a saved 1M choice remains visible after its effective capability changes");
  assert.equal(await savedOneM.isEnabled(), true, "an obsolete 1M choice must remain operable so the user can turn it off");
  assert.equal(await dialog().getByRole("button", { name: ZH.preview, exact: true }).isDisabled(), true,
    "stale 1M intent must be resolved before preview");
  await savedOneM.click();
  assert.equal(await savedOneM.getAttribute("aria-checked"), "false");
  assert.equal(await savedOneM.isDisabled(), true, "a target that lost the capability cannot turn 1M back on");
  await dialog().getByRole("button", { name: ZH.preview, exact: true }).click();
  await routeFiles.waitFor();
  const cancelledBefore = await page.evaluate(() => window.review.desktopCalls.cancelPreview.length);
  await dialog().getByRole("button", { name: "直连", exact: true }).click();
  await routeFiles.waitFor({ state: "hidden" });
  await page.waitForFunction(before => window.review.desktopCalls.cancelPreview.length > before, cancelledBefore);
  await dialog().getByRole("button", { name: ZH.cancel, exact: true }).click();
  console.log("PASS saved 1M intent remains visible after capability loss and switching a routing form cancels its preview");

  // R-A14: the global exit dialog recovers early requests and delegates every action to the coordinator.
  await page.evaluate(() => {
    window.review.mount("mcp");
    const review = window.review;
    let nextId = 0;
    const callbacks = new Map();
    review.desktopExitRequest = { intent: "quit" };
    review.desktopExitCalls = [];
    review.desktopExitFailure = null;
    review.desktopExitUnlisten = 0;
    window.__TAURI_INTERNALS__.transformCallback = callback => { const id = ++nextId; callbacks.set(id, callback); return id; };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "plugin:event|listen") { review.desktopExitHandler = args.handler; return 1; }
      if (command === "plugin:event|unlisten") { review.desktopExitUnlisten++; return; }
      if (command === "get_claude_desktop_exit_request") {
        const snapshot = structuredClone(review.desktopExitRequest);
        if (review.desktopExitHoldQuery) return new Promise(resolve => { review.desktopExitQueryResolver = () => resolve(snapshot); });
        return snapshot;
      }
      if (command === "resolve_claude_desktop_exit") {
        review.desktopExitCalls.push(args.decision);
        if (review.desktopExitFailure) throw review.desktopExitFailure;
        review.desktopExitRequest = null;
        return;
      }
      throw new Error("unexpected command in exit suite: " + command);
    };
    review.emitDesktopExit = request => {
      review.desktopExitRequest = request;
      callbacks.get(review.desktopExitHandler)({ event: "claude-desktop-exit-required", id: 1, payload: request });
    };
  });
  await page.evaluate(() => window.review.mount("desktop-exit"));
  const exit = page.locator("[data-desktop-exit-dialog]");
  await exit.getByRole("heading", { name: "退出前恢复 Claude Desktop？", exact: true }).waitFor();
  await exit.getByRole("button", { name: "保持后台运行", exact: true }).click();
  await exit.waitFor({ state: "hidden" });
  assert.deepEqual(await page.evaluate(() => window.review.desktopExitCalls), ["background"]);
  await page.evaluate(() => window.review.emitDesktopExit({ intent: "restart" }));
  await exit.getByRole("heading", { name: "重启前恢复 Claude Desktop？", exact: true }).waitFor();
  await page.evaluate(() => { window.review.desktopExitFailure = "claudeDesktop.errors.processStopTimeout"; });
  await exit.getByRole("button", { name: "停止 Desktop 并恢复重启", exact: true }).click();
  await exit.getByRole("alert").filter({ hasText: "无法结束 Claude Desktop" }).waitFor();
  assert.equal(await exit.getByRole("button", { name: "停止 Desktop 并恢复重启", exact: true }).isEnabled(), true);
  await page.evaluate(() => { window.review.desktopExitFailure = null; });
  await exit.getByRole("button", { name: "停止 Desktop 并恢复重启", exact: true }).click();
  await exit.waitFor({ state: "hidden" });
  assert.deepEqual(await page.evaluate(() => window.review.desktopExitCalls), ["background", "stopAndExit", "stopAndExit"]);
  await page.evaluate(() => window.review.mount("mcp"));
  await page.waitForFunction(() => window.review.desktopExitUnlisten === 1);
  await page.evaluate(() => {
    window.review.desktopExitRequest = { intent: "quit" };
    window.review.desktopExitHoldQuery = true;
    window.review.mount("desktop-exit");
  });
  await page.waitForFunction(() => !!window.review.desktopExitQueryResolver);
  await page.evaluate(() => {
    window.review.emitDesktopExit({ intent: "restart" });
    window.review.desktopExitQueryResolver();
    window.review.desktopExitHoldQuery = false;
  });
  await exit.getByRole("heading", { name: "重启前恢复 Claude Desktop？", exact: true }).waitFor();
  assert.equal(await exit.getByRole("heading", { name: "退出前恢复 Claude Desktop？", exact: true }).count(), 0,
    "a late startup query cannot overwrite a newer restart event");
  await exit.getByRole("button", { name: "保持后台运行", exact: true }).click();
  await exit.waitFor({ state: "hidden" });
  await page.evaluate(() => window.review.mount("mcp"));
  await page.waitForFunction(() => window.review.desktopExitUnlisten === 2);
  console.log("PASS full exit honors background choice, preserves restart intent on failure, retries and cleans its listener");

  // The updater can install once, keep the gateway in the background and restart later without reinstalling.
  await page.evaluate(() => {
    const review = window.review;
    let nextId = 0;
    const callbacks = new Map();
    const events = new Map();
    review.updaterCalls = { checks: 0, installs: 0, restarts: 0, decisions: [] };
    review.desktopExitRequest = null;
    window.__TAURI_INTERNALS__.transformCallback = callback => { const id = ++nextId; callbacks.set(id, callback); return id; };
    window.__TAURI_INTERNALS__.unregisterCallback = id => callbacks.delete(id);
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: event => events.delete(event) };
    const emit = (event, payload) => callbacks.get(events.get(event))?.({ event, id: 1, payload });
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "plugin:event|listen") { events.set(args.event, args.handler); return args.handler; }
      if (command === "plugin:event|unlisten") return;
      if (command === "get_claude_desktop_exit_request") return review.desktopExitRequest;
      if (command === "is_updater_configured") return true;
      if (command === "plugin:updater|check") {
        review.updaterCalls.checks++;
        return { rid: 1, currentVersion: "1.0.0", version: "1.0.1", body: null, rawJson: {} };
      }
      if (command === "check_update_installability") return { canInstall: true };
      if (command === "plugin:updater|download_and_install") {
        review.updaterCalls.installs++;
        args.onEvent.onmessage({ event: "Started", data: { contentLength: 100 } });
        args.onEvent.onmessage({ event: "Progress", data: { chunkLength: 100 } });
        args.onEvent.onmessage({ event: "Finished" });
        return;
      }
      if (command === "graceful_restart_for_update") {
        review.updaterCalls.restarts++;
        review.desktopExitRequest = { intent: "restart" };
        emit("claude-desktop-exit-required", review.desktopExitRequest);
        return;
      }
      if (command === "resolve_claude_desktop_exit") {
        review.updaterCalls.decisions.push(args.decision);
        const intent = review.desktopExitRequest.intent;
        review.desktopExitRequest = null;
        if (args.decision === "background") emit("claude-desktop-exit-cancelled", { intent, error: null });
        return;
      }
      if (command === "plugin:resources|close") return;
      throw new Error("unexpected updater mock command: " + command);
    };
    review.mount("desktop-updater");
  });
  await page.getByRole("button", { name: "立即更新并重启", exact: true }).waitFor();
  await page.getByRole("button", { name: "立即更新并重启", exact: true }).click();
  await exit.getByRole("heading", { name: "重启前恢复 Claude Desktop？", exact: true }).waitFor();
  await exit.getByRole("button", { name: "保持后台运行", exact: true }).click();
  await exit.waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.querySelector("[data-updater-status]")?.textContent === "idle");
  assert.equal(await page.getByText("更新已安装", { exact: true }).count(), 0,
    "keeping the gateway in the background dismisses the completed update overlay");
  await page.getByRole("button", { name: "检查测试更新", exact: true }).click();
  await page.getByRole("heading", { name: "更新已安装", exact: true }).waitFor();
  await page.getByRole("button", { name: "稍后重启", exact: true }).click();
  await page.getByRole("button", { name: "检查测试更新", exact: true }).click();
  await page.getByRole("button", { name: "立即重启", exact: true }).click();
  await exit.getByRole("button", { name: "停止 Desktop 并恢复重启", exact: true }).click();
  await exit.waitFor({ state: "hidden" });
  assert.deepEqual(await page.evaluate(() => window.review.updaterCalls),
    { checks: 1, installs: 1, restarts: 2, decisions: ["background", "stopAndExit"] },
    "a ready update must request only the coordinated restart without downloading or installing twice");
  await page.evaluate(() => window.review.mount("mcp"));
  console.log("PASS an installed update supports background cancellation and a later coordinated restart without reinstalling");

  // Saved Desktop configurations must survive closing the form and one-click
  // apply must commit exactly the validated snapshot, without storing a key.
  await reset("groups");
  await install();
  await page.evaluate(() => window.review.mount("mcp"));
  await page.evaluate(() => window.review.mount("groups"));
  await actionButton(ZH.change).waitFor();
  await actionButton(ZH.change).click();
  await dialog().waitFor();
  const savedName = "Desktop saved smoke";
  await dialog().locator("li").filter({ has: modelCheckbox(SHORT_MODEL_ID) })
    .getByRole("checkbox", { name: "1M", exact: true }).uncheck();
  await dialog().getByRole("button", { name: "保存当前配置", exact: true }).click();
  await dialog().getByRole("textbox", { name: "配置名称", exact: true }).fill(savedName);
  await dialog().getByRole("button", { name: "保存", exact: true }).click();
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem("agentharbor.claude-desktop-profiles.v1")));
  assert.equal(stored.length, 1);
  assert.equal(stored[0].name, savedName);
  assert.equal(stored[0].models.length, 2);
  assert.doesNotMatch(JSON.stringify(stored), /fixture-upstream|sk-SENTINEL|apiKey|accessToken/);
  await dialog().getByRole("button", { name: ZH.cancel, exact: true }).click();
  await actionButton(ZH.change).click();
  await dialog().locator("#desktop-saved-config").click();
  await page.getByRole("option", { name: `${savedName} · 直连`, exact: true }).click();
  await dialog().getByRole("button", { name: "一键应用", exact: true }).click();
  await dialog().waitFor({ state: "hidden" });
  const savedCalls = await page.evaluate(() => window.review.desktopCalls);
  assert.equal(savedCalls.preview.length, 1);
  assert.equal(savedCalls.commit.length, 1);
  assert.deepEqual(savedCalls.commit[0].request, savedCalls.preview[0].request);
  console.log("PASS a saved Desktop configuration survives reopening and one-click applies its previewed request");

  await page.evaluate(() => { delete window.__TAURI_INTERNALS__.invoke; });
  await page.setViewportSize({ width: 1200, height: 700 });
}
