import { storageKeys } from "@/lib/storage-keys";

export interface CustomInstructionTemplate {
  code: string;
  title: string;
  summary: string;
  body: string;
  tags: string[];
  applyCount?: number;
  source: "builtin" | "saved";
  category?: string;
}

export const builtinCustomInstructionTemplates: CustomInstructionTemplate[] = [
  {
    code: "zh-structured",
    title: "高密度中文工程助手",
    summary: "默认中文、先结论后分析，适合高频开发协作。",
    tags: ["通用", "中文", "高密度"],
    source: "builtin",
    body: `默认使用简体中文。\n先给结论，再讲原因，再给下一步。\n回答保持结构化、高信息密度、少废话。\n修复问题前先核查主链与副作用，优先最小必要改动。`,
  },
  {
    code: "review-first",
    title: "严格代码审查",
    summary: "优先输出问题、风险和测试缺口，适合审查与复盘。",
    tags: ["审查", "严格模式", "风险"],
    source: "builtin",
    body: `当任务是代码审查时，先列出问题，再讲风险，再给结论。\n按严重性排序，优先指出行为回归、边界条件、缺少校验和测试缺口。\n没有问题时，明确说明“未发现明确缺陷”，并补充残留风险。`,
  },
  {
    code: "safe-fix",
    title: "安全修复优先",
    summary: "修 bug 前先核查副作用，适合稳定链路和线上功能。",
    tags: ["修复", "稳定性", "副作用"],
    source: "builtin",
    body: `修复前先说明当前流程、根因判断、影响范围和可能副作用。\n默认只改最小必要层，不顺手重构稳定链。\n完成后说明验证路径、受影响模块和是否存在残留边界。`,
  },
  {
    code: "frontend-polish",
    title: "前端体验优化",
    summary: "关注 UI 语义、交互清晰度和设计一致性。",
    tags: ["前端", "体验", "设计"],
    source: "builtin",
    body: `当前端任务涉及设计和交互时，优先收口信息层级、控件语义和状态反馈。\n保留现有设计系统，不随意引入新的视觉语言。\n默认说明“当前流程 → 问题 → 优化后流程 → 用户体感变化”。`,
  },
  {
    code: "implementation",
    title: "日常开发",
    summary: "先读项目约定，再实现并验证；适合多数开发任务。",
    tags: ["开发", "验证"],
    source: "builtin",
    category: "development",
    body: "开始修改前阅读项目指令、相关实现和测试，沿用现有约定。\n先明确任务目标；复杂任务列出简短步骤，简单任务直接处理。\n优先实现当前需要的行为，避免无关重构和新增依赖。\n完成后运行与改动有关的构建或测试，说明结果与未验证项。",
  },
  {
    code: "test-driven",
    title: "测试与回归",
    summary: "先复现问题，用行为测试保护修复。",
    tags: ["测试", "回归"],
    source: "builtin",
    category: "development",
    body: "修复缺陷时先定位可复现的输入和预期行为。\n针对缺陷补充能在修复前失败的测试，覆盖相关边界。\n以对外行为为断言，避免只重复实现细节。\n测试通过后检查回归；不声明未实际运行的验证。",
  },
  {
    code: "refactor",
    title: "渐进式重构",
    summary: "保留对外行为，逐步收口职责和依赖。",
    tags: ["重构", "兼容"],
    source: "builtin",
    category: "development",
    body: "重构前明确需要解决的耦合或重复问题，说明现有对外契约。\n保留已支持的行为、配置与数据格式；有必要迁移时显式处理兼容。\n按可验证的小步骤调整职责，避免引入没有当前消费者的抽象。\n完成后复查调用方和回归测试，记录实际改变的行为。",
  },
  {
    code: "documentation",
    title: "文档与交付",
    summary: "围绕真实功能写说明、示例和限制。",
    tags: ["文档", "交付"],
    source: "builtin",
    category: "writing",
    body: "按读者要完成的任务组织文档，先说明用途，再给最短可运行示例。\n核对代码和配置后再描述功能，不把计划写成已完成能力。\n明确前置条件、输入输出、常见错误与处理办法。\n使用项目实际术语，删除重复说明与无法验证的宣传性表述。",
  },
];

const english: Record<string, [string, string, string]> = {
  "zh-structured": [
    "Concise engineering assistant",
    "Lead with the outcome and concrete next steps.",
    "Use concise, structured language.\nLead with the outcome, then explain evidence and next steps.\nBefore fixing a problem, inspect its cause and side effects; prefer the smallest useful change.",
  ],
  "review-first": [
    "Code review",
    "Prioritize defects, regressions and missing coverage.",
    "For code reviews, list findings before the conclusion.\nSort by severity and include file locations, concrete triggers and consequences.\nPrioritize regressions, boundary conditions and missing validation.\nWhen no defects are found, state what was checked and any remaining gaps.",
  ],
  "safe-fix": [
    "Reliable bug fixes",
    "Inspect the cause and protect working behavior.",
    "Trace the failing behavior and identify its cause before changing code.\nPrefer a focused fix; preserve working paths and avoid unrelated refactoring.\nVerify the failure case and relevant regressions, then report the actual results.",
  ],
  "frontend-polish": [
    "Interface refinement",
    "Improve interactions within the existing design system.",
    "Prioritize clear hierarchy, meaningful controls and visible state feedback.\nPreserve the existing design system and use its theme tokens.\nCheck keyboard use, loading, error and empty states, and narrow layouts.",
  ],
  implementation: [
    "Everyday development",
    "Read conventions, implement and verify.",
    "Read project instructions, related code and tests before editing.\nClarify the goal; use a short plan for complex work and act directly on simple tasks.\nImplement current requirements without unrelated refactors or unnecessary dependencies.\nRun relevant tests or builds and report results and unverified areas.",
  ],
  "test-driven": [
    "Testing and regressions",
    "Reproduce defects and protect fixes with behavioral tests.",
    "Identify reproducible inputs and expected behavior before fixing a defect.\nAdd a test that fails before the fix and covers relevant boundaries.\nAssert public behavior rather than duplicating implementation.\nCheck regressions and only claim checks actually performed.",
  ],
  refactor: [
    "Incremental refactoring",
    "Reduce coupling while preserving public behavior.",
    "Identify the coupling or duplication to address and the existing public contract.\nPreserve supported behavior, configuration and data formats; handle any necessary migration explicitly.\nRefactor in small verifiable steps; avoid abstractions with no current consumer.\nReview callers and regression tests, and document behavior changes.",
  ],
  documentation: [
    "Documentation and delivery",
    "Write usable examples based on implemented behavior.",
    "Organize documentation around the reader's task and provide a minimal working example.\nVerify code and configuration before describing a feature.\nState prerequisites, inputs, outputs and actionable error handling.\nUse project terminology and remove repeated or unverifiable claims.",
  ],
};

export function instructionTemplates(
  language: string,
): CustomInstructionTemplate[] {
  return builtinCustomInstructionTemplates.map((item) => {
    const translated = language.startsWith("en")
      ? english[item.code]
      : undefined;
    return {
      ...item,
      category:
        item.category ??
        (item.code === "review-first" ? "review" : "development"),
      ...(translated
        ? {
            title: translated[0],
            summary: translated[1],
            body: translated[2],
            tags: [],
          }
        : {}),
    };
  });
}

function presetKey(source: "codex" | "claudeCode") {
  return `${storageKeys.instructionPresets}.${source}`;
}

export function loadSavedTemplates(
  source: "codex" | "claudeCode",
): CustomInstructionTemplate[] {
  try {
    const items: unknown = JSON.parse(
      localStorage.getItem(presetKey(source)) ?? "[]",
    );
    if (!Array.isArray(items)) return [];
    const seen = new Set<string>();
    return items
      .filter(
        (item) =>
          item &&
          typeof item.code === "string" &&
          item.code.startsWith("saved-") &&
          !seen.has(item.code) &&
          typeof item.title === "string" &&
          item.title.length <= 80 &&
          typeof item.body === "string" &&
          typeof item.summary === "string" &&
          Array.isArray(item.tags) &&
          item.tags.every((tag: unknown) => typeof tag === "string") &&
          item.source === "saved" &&
          seen.add(item.code),
      )
      .slice(0, 40);
  } catch {
    return [];
  }
}

export function saveTemplates(
  source: "codex" | "claudeCode",
  templates: CustomInstructionTemplate[],
) {
  localStorage.setItem(presetKey(source), JSON.stringify(templates));
}
