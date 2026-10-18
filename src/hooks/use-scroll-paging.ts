import { useEffect, useState } from "react";

/** The nearest ancestor that actually scrolls vertically. An `overflow-x`
 *  wrapper computes `overflow-y: auto` too, but never grows past its content,
 *  so it would report the end as always in view. */
function scrollRoot(element: Element): Element | null {
  for (let node = element.parentElement; node; node = node.parentElement) {
    const { overflowY } = getComputedStyle(node);
    if ((overflowY === "auto" || overflowY === "scroll") && node.scrollHeight > node.clientHeight) return node;
  }
  return null;
}

/** Shows a long list a page at a time; the next page arrives as the list
 *  scrolls near its end. Put `endRef` on an element after the last row while
 *  `more` is true, and call `reset` when the filter changes. */
export function useScrollPaging(total: number, page: number) {
  const [limit, setLimit] = useState(page);
  const [end, setEnd] = useState<Element | null>(null);
  const more = total > limit;
  // Re-armed per page: a page that still leaves the end in view loads the next one too.
  useEffect(() => {
    if (!more || !end || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting)) setLimit(value => value + page);
    }, { root: scrollRoot(end), rootMargin: "0px 0px 160px 0px" });
    observer.observe(end);
    return () => observer.disconnect();
  }, [more, end, limit, page]);
  return { limit, more, endRef: setEnd, reset: () => setLimit(page) };
}
