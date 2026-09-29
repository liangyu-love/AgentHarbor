const officialHosts = new Set([
  "api.deepseek.com", "api.stepfun.com", "api.stepfun.ai", "api.siliconflow.cn",
  "api.siliconflow.com", "openrouter.ai", "api.novita.ai", "api.iiiiitoken.com", "api.minimaxi.com", "api.minimax.io",
]);

export function isOfficialBalanceSite(baseUrl: string): boolean {
  try {
    const url = new URL(baseUrl);
    return officialHosts.has(url.hostname.toLowerCase())
      || (url.hostname === "opencode.ai" && /^\/zen\/go(?:\/v1)?\/?$/.test(url.pathname))
      || (["open.bigmodel.cn", "api.z.ai"].includes(url.hostname) && url.pathname.startsWith("/api/coding/"))
      || (url.hostname === "api.kimi.com" && url.pathname.startsWith("/coding"));
  } catch { return false; }
}

export function balanceSourceName(source: string | null): string {
  const names: Record<string, string> = {
    newapi: "New API", sub2api: "Sub2API", deepseek: "DeepSeek", stepfun: "StepFun",
    siliconflow: "SiliconFlow", siliconflowIntl: "SiliconFlow International",
    openrouter: "OpenRouter", novita: "Novita AI", opencodeGo: "OpenCode Go", kimiCoding: "Kimi Coding", minimaxPlan: "MiniMax Plan", glmPlan: "GLM Coding Plan", custom: "Custom", apiKeyQuota: "API Key",
  };
  return source ? names[source] ?? source : "";
}

export const BALANCE_SCRIPT_TEMPLATE = `({
  // 余额请求配置 (变量占位符: {{baseUrl}}, {{apiKey}}, {{accessToken}}, {{userId}})
  request: {
    url: "{{baseUrl}}/v1/usage",
    method: "GET",
    headers: { Authorization: "Bearer {{apiKey}}" }
  },
  // 响应解析器：提取剩余额度与单位 (可选 used, total, resetsAt)
  extractor: function(response) {
    const data = response?.data ?? response;
    const remaining = data?.remaining ?? data?.quota?.remaining ?? data?.balance;
    if (remaining == null || !Number.isFinite(Number(remaining))) {
      throw new Error("Missing balance");
    }
    return {
      isValid: data?.is_active ?? data?.isValid ?? true,
      remaining: Number(remaining),
      unit: data?.unit ?? data?.quota?.unit ?? "USD"
    };
  }
})`;
