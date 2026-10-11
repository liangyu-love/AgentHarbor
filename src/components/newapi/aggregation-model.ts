import { sameModel } from "@/lib/same-model";
import type { AggregateLive, AggregateMember, AggregateModel, AggregateView } from "@/types/aggregation";
import type { NewApiProvider } from "@/types/newapi";

export const initialAggregate = (): AggregateModel => ({
  id: "",
  publicModelId: "",
  displayName: "",
  enabled: true,
  members: [],
  strategy: "roundRobin",
  affinity: "session",
  retryPolicy: { maxAttempts: 2, failureThreshold: 3 },
  compression: { mode: "inherit" },
  firstTokenTimeoutSecs: 0,
});

/** The models a relay card offers an aggregate: its included models and its default. */
export const modelsOf = (provider: NewApiProvider | undefined) =>
  provider
    ? provider.models
        .filter((model) => model.included)
        .map((model) => model.id)
        .concat(!provider.models.some((model) => model.id === provider.model) && provider.model ? [provider.model] : [])
    : [];

// Members are tried tier by tier: the lowest priority number first, the next
// tier only when every member above is unavailable. Within a tier the strategy
// rotates, so the order inside a tier carries no meaning.
export const tiersOf = (members: AggregateMember[]) =>
  [...new Set(members.map((member) => member.priority))].sort((a, b) => a - b);

/** Saved priorities are the tier positions, so they read 0, 1, 2… */
export const normalizeTiers = (members: AggregateMember[]) => {
  const tiers = tiersOf(members);
  return members.map((member) => ({ ...member, priority: tiers.indexOf(member.priority) }));
};

/** The priority a member takes when it moves one tier up or down. */
export const shiftedPriority = (members: AggregateMember[], member: AggregateMember, direction: -1 | 1): number | null => {
  const tiers = tiersOf(members);
  const index = tiers.indexOf(member.priority);
  if (direction === -1) return index > 0 ? tiers[index - 1] : null;
  if (index < tiers.length - 1) return tiers[index + 1];
  // From the last tier a member starts a new one, unless it is that tier alone.
  return members.filter((other) => other.priority === member.priority).length > 1 ? member.priority + 1 : null;
};

export type MemberState = "hit" | "cooling" | "probe" | "standby" | "off" | "down";

/** What a member is doing right now, from the saved view and the live runtime. */
export function memberState(model: AggregateView, member: AggregateMember, live: AggregateLive | undefined) {
  const status = model.memberStates.find((state) => state.memberId === member.id);
  const runtime = live?.members.find((entry) => entry.memberId === member.id);
  const state: MemberState = !member.enabled ? "off"
    : !status?.available ? "down"
    : runtime?.coolingMs ? "cooling"
    : runtime?.tripped ? "probe"
    : live?.preferredMemberId === member.id ? "hit"
    : "standby";
  return { state, status, runtime };
}

export interface MergeSuggestion {
  key: string;
  publicModelId: string;
  members: { providerId: string; providerName: string; modelId: string }[];
}

// Models that more than one usable relay serves under the same name, however
// each spells it, and that no aggregate covers yet. Most-served first.
export function mergeSuggestions(providers: NewApiProvider[], aggregates: AggregateModel[]): MergeSuggestion[] {
  const groups = new Map<string, MergeSuggestion & { spellings: string[]; official?: string }>();
  for (const provider of providers) {
    if (!provider.enabled || (!provider.hasApiKey && !provider.accountSource)) continue;
    for (const modelId of modelsOf(provider)) {
      const key = sameModel(modelId);
      if (!key) continue;
      const group = groups.get(key) ?? { key, publicModelId: "", members: [], spellings: [] };
      groups.set(key, group);
      if (group.members.some((member) => member.providerId === provider.id)) continue;
      group.members.push({ providerId: provider.id, providerName: provider.name, modelId });
      group.spellings.push(modelId.slice(modelId.lastIndexOf("/") + 1));
      const reference = provider.models.find((model) => model.id === modelId)?.catalogRef?.modelId;
      if (reference && sameModel(reference) === key) group.official ??= reference;
    }
  }
  const taken = new Set(aggregates.map((aggregate) => sameModel(aggregate.publicModelId)));
  const covered = (group: MergeSuggestion) => aggregates.some((aggregate) =>
    group.members.every((member) => aggregate.members.some((existing) =>
      existing.providerId === member.providerId && existing.actualModelId === member.modelId)));
  return [...groups.values()]
    .filter((group) => group.members.length >= 2 && !taken.has(group.key) && !covered(group))
    .map(({ key, members, spellings, official }) => ({
      key,
      members,
      // The catalog's spelling when a card has one, else the plainest spelling.
      publicModelId: official ?? [...spellings].sort((a, b) =>
        a.length - b.length || Number(a !== a.toLowerCase()) - Number(b !== b.toLowerCase()))[0],
    }))
    .sort((a, b) => b.members.length - a.members.length || a.publicModelId.localeCompare(b.publicModelId));
}

export const aggregateFromSuggestion = (suggestion: MergeSuggestion): AggregateModel => ({
  ...initialAggregate(),
  publicModelId: suggestion.publicModelId,
  displayName: suggestion.publicModelId,
  members: suggestion.members.map((member) => ({
    id: crypto.randomUUID(),
    providerId: member.providerId,
    actualModelId: member.modelId,
    enabled: true,
    priority: 0,
    weight: 1,
  })),
});
