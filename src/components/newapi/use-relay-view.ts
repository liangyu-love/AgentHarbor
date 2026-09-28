import { useEffect, useMemo, useState } from "react";
import { storageKeys } from "@/lib/storage-keys";
import { groupProviders, hostOf, type RelayGroup } from "@/components/newapi/relay-groups";
import type { NewApiProvider } from "@/types/newapi";

// View state for the relay cards: filter chip, saved group aliases and the
// per-group member cursor / stack expansion. Lives above the cards so it
// survives filtering them out and back in.
export function useRelayView(providers: NewApiProvider[], loaded: boolean) {
  const [filter, setFilter] = useState<string>("all");
  const [groupAliases, setGroupAliases] = useState<Record<string, string>>(() => {
    try {
      const saved = localStorage.getItem(storageKeys.relayGroupAliases);
      return saved ? JSON.parse(saved) as Record<string, string> : {};
    } catch {
      return {};
    }
  });
  const [aliasGroup, setAliasGroup] = useState<RelayGroup | null>(null);
  const [aliasDraft, setAliasDraft] = useState("");
  const [groupMemberSelect, setGroupMemberSelect] = useState<Record<string, string>>({});
  const [groupMemberDirection, setGroupMemberDirection] = useState<Record<string, "prev" | "next">>({});
  const [expandedGroups, setExpandedGroups] = useState<Record<string, boolean>>({});

  const groups = useMemo(() => groupProviders(providers), [providers]);
  useEffect(() => {
    if (loaded && filter !== "all" && !groups.some(group => group.key === filter)) {
      setFilter("all");
    }
  }, [filter, groups, loaded]);
  const shown = filter === "all" ? groups : groups.filter(group => group.key === filter);

  const groupLabel = (group: RelayGroup, provider = group.providers[0]) => groupAliases[group.key] ?? hostOf(provider);
  const renameGroup = (group: RelayGroup) => {
    const current = groupAliases[group.key] ?? hostOf(group.providers[0]);
    setAliasGroup(group);
    setAliasDraft(current);
  };
  const saveGroupAlias = () => {
    if (!aliasGroup) return;
    const next = aliasDraft.trim();
    if (!next) return;
    const updated = { ...groupAliases, [aliasGroup.key]: next };
    setGroupAliases(updated);
    localStorage.setItem(storageKeys.relayGroupAliases, JSON.stringify(updated));
    setAliasGroup(null);
  };
  const selectMember = (group: RelayGroup, id: string, direction: "prev" | "next") => {
    setGroupMemberDirection(prev => ({ ...prev, [group.key]: direction }));
    setGroupMemberSelect(prev => ({ ...prev, [group.key]: id }));
  };
  const setExpanded = (group: RelayGroup, expanded: boolean) =>
    setExpandedGroups(prev => ({ ...prev, [group.key]: expanded }));
  const toggleExpanded = (group: RelayGroup) =>
    setExpandedGroups(prev => ({ ...prev, [group.key]: !prev[group.key] }));

  return {
    filter, setFilter, groups, shown, groupLabel,
    aliasGroup, setAliasGroup, aliasDraft, setAliasDraft, renameGroup, saveGroupAlias,
    memberSelection: groupMemberSelect, memberDirection: groupMemberDirection, expandedGroups,
    selectMember, setExpanded, toggleExpanded,
  };
}

export type RelayView = ReturnType<typeof useRelayView>;
