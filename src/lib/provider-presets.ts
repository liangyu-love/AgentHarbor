import type { ModelProtocol, NewApiModel } from "@/types/newapi";

// Connection presets, not a hard-coded model catalog. Fetching /models remains
// available; Go's seed models include the per-model protocol its API requires.
export interface ProviderPreset {
  name: string;
  baseUrl: string;
  protocol: ModelProtocol;
  models?: NewApiModel[];
  category?: "direct" | "coding" | "aggregator" | "official";
  description?: string;
}

const model = (id: string, protocol: ModelProtocol): NewApiModel => ({
  id,
  displayName: id,
  protocol,
  included: true,
  contextWindow: null,
  maxOutputTokens: null,
  endpointTypes: [],
  metadataSource: "local",
});

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    name: "DeepSeek",
    baseUrl: "https://api.deepseek.com/v1",
    protocol: "chatCompletions",
    category: "direct",
    description: "DeepSeek 官方直连 (V3 / R1 推理与代码模型)",
  },
  {
    name: "OpenCode Go",
    baseUrl: "https://opencode.ai/zen/go/v1",
    protocol: "chatCompletions",
    category: "coding",
    description: "OpenCode 多模型混合协议专线",
    models: [
      model("glm-5.3", "chatCompletions"),
      model("kimi-k3", "chatCompletions"),
      model("minimax-m3", "anthropicMessages"),
      model("gpt-6-luna", "responses"),
    ],
  },
  {
    name: "OpenCode Zen",
    baseUrl: "https://opencode.ai/zen/v1",
    protocol: "chatCompletions",
    category: "coding",
    description: "OpenCode Zen 标准模型端点",
  },
  {
    name: "OpenRouter",
    baseUrl: "https://openrouter.ai/api/v1",
    protocol: "chatCompletions",
    category: "aggregator",
    description: "OpenRouter 全球模型聚合网关",
  },
  {
    name: "SiliconFlow CN",
    baseUrl: "https://api.siliconflow.cn/v1",
    protocol: "chatCompletions",
    category: "aggregator",
    description: "硅基流动国内模型平台",
  },
  {
    name: "SiliconFlow Global",
    baseUrl: "https://api.siliconflow.com/v1",
    protocol: "chatCompletions",
    category: "aggregator",
    description: "SiliconFlow 国际云端平台",
  },
  {
    name: "Kimi Coding",
    baseUrl: "https://api.kimi.com/coding/v1",
    protocol: "anthropicMessages",
    category: "coding",
    description: "Moonshot Kimi 代码专用模型",
    models: [model("kimi-for-coding", "anthropicMessages")],
  },
  {
    name: "MiniMax CN",
    baseUrl: "https://api.minimaxi.com/anthropic",
    protocol: "anthropicMessages",
    category: "coding",
    description: "MiniMax 国内 Anthropic 兼容端点",
    models: [model("MiniMax-M3", "anthropicMessages")],
  },
  {
    name: "MiniMax Global",
    baseUrl: "https://api.minimax.io/anthropic",
    protocol: "anthropicMessages",
    category: "coding",
    description: "MiniMax 国际 Anthropic 兼容端点",
    models: [model("MiniMax-M3", "anthropicMessages")],
  },
  {
    name: "GLM Coding Plan",
    baseUrl: "https://open.bigmodel.cn/api/coding/paas/v4",
    protocol: "chatCompletions",
    category: "coding",
    description: "智谱 GLM 编程套餐专属端点",
  },
  {
    name: "Z.AI Coding Plan",
    baseUrl: "https://api.z.ai/api/coding/paas/v4",
    protocol: "chatCompletions",
    category: "coding",
    description: "智谱 Z.AI 编程套餐端点",
  },
  {
    name: "MiMo API",
    baseUrl: "https://api.xiaomimimo.com/v1",
    protocol: "chatCompletions",
    category: "coding",
    description: "小米 MiMo 官方 API",
  },
  {
    name: "MiMo Token Plan",
    baseUrl: "https://token-plan-cn.xiaomimimo.com/v1",
    protocol: "chatCompletions",
    category: "coding",
    description: "小米 MiMo Token Plan 专用入口；模型清单可按密钥拉取",
  },
  {
    name: "Command Code",
    baseUrl: "https://api.commandcode.ai/provider/v1",
    protocol: "chatCompletions",
    category: "coding",
    description: "Command Code 官方 Coding Plan / Provider API",
  },
  {
    name: "StepFun",
    baseUrl: "https://api.stepfun.com/v1",
    protocol: "chatCompletions",
    category: "direct",
    description: "阶跃星辰 Step 官方端点",
  },
  {
    name: "Novita AI",
    baseUrl: "https://api.novita.ai/openai/v1",
    protocol: "chatCompletions",
    category: "aggregator",
    description: "Novita AI 云端推理",
  },
  {
    name: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    protocol: "responses",
    category: "official",
    description: "OpenAI 官方 Responses 接口",
  },
  {
    name: "Anthropic",
    baseUrl: "https://api.anthropic.com/v1",
    protocol: "anthropicMessages",
    category: "official",
    description: "Anthropic 官方 Messages 接口",
  },
];
