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
  hintManaged: "Claude Desktop 正在直接使用一张已保存的中转卡片。",
  warningRequiresRestart: "请完全退出 Claude Desktop（包括托盘进程）后重新启动，设置才会生效。",
  change: "切换 / 更新",
  reapply: "重新应用",
  restorePrevious: "恢复接管前配置",
  switchOfficial: "切换官方模式",
  release: "释放管理",
  conflictReason: "请先解决冲突或按当前状态重新接管。",
  needModels: "请至少选择一个模型。",
  busyHint: "正在处理，请稍候。",
  dialogTitle: "配置 Claude Desktop 直连",
  providerDisabled: "该卡片未加入路由池；直连不要求启用它，本次选择也不会启用。",
  noInferenceKey: "该卡片没有推理密钥；请先在卡片上补充，余额 token 不能作为推理凭据。",
  labelPlaceholder: "显示名称（可选）",
  oneMAuto: "1M：自动",
  oneMOff: "1M：关闭",
  messagesCheck: "核对 Messages 兼容性",
  preview: "预览",
  apply: "应用",
  cancel: "取消",
  confirm: "确认",
  restoreTitle: "恢复接管前的配置？",
  restoreDescription: "将撤回 AgentHarbor 写入的部署模式和配置，并恢复之前的生效配置选择；外部配置与无关键都会保留。",
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
  staleReasons: [],
  conflicts: [],
  recoveryState: null,
  pendingRecovery: false,
  warnings: ["claudeDesktop.warnings.requiresRestart"],
  lastAppliedAt: 1_790_000_000,
  restartRequired: true,
  actions: [
    { action: "applyDirect", enabled: true, disabledReason: null },
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
      reason: "no_inference_key", endpoint: null, models: [] },
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
    review.desktopCalls = { candidates: [], preview: [], commit: [], probe: [], other: [] };
    review.desktopHoldPreview = false;
    review.desktopPreviewResolvers = [];
    review.desktopPreviewFailure = null;
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "get_claude_desktop_status") return clone(review.desktopStatus);
      if (command === "list_claude_desktop_candidates") {
        review.desktopCalls.candidates.push(clone(args));
        return clone(review.desktopCandidates);
      }
      if (command === "preview_claude_desktop_change") {
        review.desktopCalls.preview.push(clone(args));
        if (review.desktopPreviewFailure) throw review.desktopPreviewFailure;
        if (review.desktopHoldPreview) {
          return new Promise(resolve => review.desktopPreviewResolvers.push(() => resolve(clone(review.desktopPreview))));
        }
        return clone(review.desktopPreview);
      }
      if (command === "commit_claude_desktop_change") {
        review.desktopCalls.commit.push(clone(args));
        return clone(review.desktopApplied);
      }
      if (command === "probe_claude_desktop_provider") {
        review.desktopCalls.probe.push(clone(args));
        return clone(review.desktopProbe);
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
      hint: text("p.break-words.text-sm"),
      models: text("p.break-words.font-mono.text-xs"),
      modelsStyle: styles("p.break-words.font-mono.text-xs"),
      path: text("p.break-all.font-mono"),
      pathStyle: styles("p.break-all.font-mono"),
      statuses: [...node.querySelectorAll('[role="status"]')].map(element => element.textContent.trim()),
      alerts: node.querySelectorAll('[role="alert"]').length,
      buttons: [...node.querySelectorAll("button")].map(button => ({ text: button.innerText.trim(),
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
  assert.equal(content.hint, ZH.hintManaged);
  assert.equal(content.alerts, 0);
  // Both the restart warning and the reason a manage button is disabled are live regions.
  assert.deepEqual(content.statuses, [ZH.warningRequiresRestart, ZH.conflictReason],
    "warnings and disabled-action reasons must render as role=status");
  const badgeTexts = content.badgeRow.join(" | ");
  for (const badge of [ZH.stateManaged, ZH.modeDirect, "v1.2.3", ZH.requiresRestart]) {
    assert.ok(badgeTexts.includes(badge), "missing card badge " + badge + " in " + badgeTexts);
  }
  assert.match(content.innerText, /本地恢复备份: 3/);
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
      assert.deepEqual(metrics.buttons.filter(button => !button.disabled).map(button => button.text),
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
    // NOTE: the ineligible-provider badge (shrink-0 with a full sentence) currently spills
    // ~24px past this scroll area at a ~660px dialog and adds an inner horizontal scrollbar.
    // That lives in src/ and is reported as a finding, not asserted here.
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
      providerButtons: [...node.querySelectorAll("button[aria-pressed]")].map(button => ({ text: button.innerText.trim().split("\n")[0],
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
  assert.equal(dialogSurface.messagesCheckDisabled, true, "the Messages check needs a selected model");
  assert.equal(dialogSurface.probeDisabled, false);
  assert.ok((await dialog().getByRole("button", { name: /Relay Beta/ }).innerText()).includes(ZH.providerDisabled),
    "an ineligible provider must explain why it cannot be picked");
  assert.equal(await dialog().getByRole("button", { name: ZH.preview, exact: true }).isDisabled(), true,
    "preview must stay disabled until a model is selected");
  assert.equal(await dialog().locator('[role="status"]').filter({ hasText: ZH.needModels }).count(), 1,
    "a disabled preview must state its reason");

  // Model rows: checkbox id <-> label for pairing, then Tab reachability and Space toggling.
  const modelIds = [LONG_MODEL_ID, SHORT_MODEL_ID];
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
  assert.equal(await dialog().getByText("chat-only-model", { exact: true }).count(), 0, "non-selectable models must stay hidden");

  const tabWalk = [];
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

  // Select both models, then drive the 1M Select on the long row.
  for (const modelId of modelIds) await modelCheckbox(modelId).check();
  assert.equal(await dialog().getByRole("button", { name: ZH.messagesCheck, exact: true }).isEnabled(), true);
  const longRow = dialog().locator("li").filter({ has: page.locator('[id="desktop-model-' + LONG_MODEL_ID + '"]') });
  const oneM = longRow.getByRole("combobox");
  assert.equal(await oneM.count(), 1, "a selected model must expose the 1M select");
  assert.equal(await longRow.getByRole("textbox", { name: ZH.labelPlaceholder, exact: true }).getAttribute("maxlength"), "64");
  await oneM.click();
  assert.deepEqual(await page.getByRole("option").allInnerTexts(), [ZH.oneMAuto, ZH.oneMOff]);
  await page.getByRole("option", { name: ZH.oneMOff, exact: true }).click();
  assert.equal((await oneM.innerText()).trim(), ZH.oneMOff);
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
  await page.evaluate(() => { window.review.desktopPreviewResolvers.shift()(); });
  await dialog().getByText("normalConfig", { exact: true }).waitFor();
  assert.equal(await fieldset.evaluate(node => node.disabled), false, "the fieldset must unlock when the request settles");
  assert.deepEqual(await page.evaluate(() => window.review.desktopCalls.preview[0]), {
    action: "applyDirect",
    request: {
      providerId: "group-a",
      models: [
        // The long row was switched to "1M: off" through the select above.
        { modelId: LONG_MODEL_ID, labelOverride: LONG_MODEL_ID, oneMMode: "off" },
        { modelId: SHORT_MODEL_ID, labelOverride: SHORT_MODEL_ID, oneMMode: "auto" },
      ],
      paths: { normalRoot: NORMAL_ROOT, thirdPartyRoot: THIRD_PARTY_ROOT },
      rebaseline: false,
    },
  });
  const previewCards = await dialog().evaluate(node => {
    const files = [...node.querySelectorAll("div.rounded-lg.border")].map(entry => entry.innerText.trim());
    return { files, primary: files.find(text => text.includes("normalConfig")) ?? "",
      statuses: [...node.querySelectorAll('[role="status"]')].map(element => element.textContent.trim()),
      pathsFit: [...node.querySelectorAll("p.font-mono")].every(entry => entry.scrollWidth <= entry.clientWidth + 1),
      overflow: node.scrollWidth - node.clientWidth };
  });
  assert.ok(previewCards.primary.includes("normalConfig"), "preview file cards are missing: " + JSON.stringify(previewCards.files));
  assert.ok(previewCards.primary.includes("update"), JSON.stringify(previewCards.primary));
  assert.ok(previewCards.primary.includes(LONG_PATH), "the preview card must show the managed file path");
  assert.ok(previewCards.primary.includes("deploymentMode: 1p → 3p"), "the preview card must show managed changes");
  assert.ok(previewCards.files.some(text => text.includes("thirdPartyConfig") && text.includes("create")));
  for (const status of [ZH.warningRequiresRestart, ZH.previewRestart]) {
    assert.ok(previewCards.statuses.includes(status), "missing preview status " + status + " in " + JSON.stringify(previewCards.statuses));
  }
  assert.equal(await dialog().getByText(ZH.externalProfiles + ": 1", { exact: true }).count(), 1,
    "the preview must report the preserved external profiles");
  assert.equal(previewCards.pathsFit, true, "a long preview path must not spill out of its card");
  assert.ok(previewCards.overflow <= 1, "the preview must not widen the dialog: " + previewCards.overflow);
  console.log("PASS the dialog locks while busy, sends the model/1M/paths selection and renders the preview file cards");

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
  await confirm.getByRole("heading", { name: ZH.restoreTitle, exact: true }).waitFor();
  assert.equal(await confirm.getByText(ZH.restoreDescription, { exact: true }).count(), 1);
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

  await page.evaluate(() => { delete window.__TAURI_INTERNALS__.invoke; });
  await page.setViewportSize({ width: 1200, height: 700 });
}
