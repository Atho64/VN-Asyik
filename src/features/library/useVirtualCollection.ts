import { useLayoutEffect, useState, useSyncExternalStore } from "react";
import {
  Virtualizer,
  elementScroll,
  observeElementOffset,
  observeElementRect,
  type VirtualItem,
  type VirtualizerOptions,
} from "@tanstack/react-virtual";

type ScrollElement = HTMLElement | HTMLDivElement;
type Options = Omit<
  VirtualizerOptions<ScrollElement, HTMLDivElement>,
  "scrollToFn" | "observeElementRect" | "observeElementOffset" | "onChange"
>;

interface Snapshot {
  items: readonly VirtualItem[];
  total: number;
}

/**
 * TanStack's React hook exposes a mutable instance, which React Compiler cannot
 * memoize safely. This adapter publishes immutable range snapshots instead.
 * Lifecycle/observers follow react-virtual 3.14.21's src/index.tsx; the package
 * exports the core and observer APIs. No dependency or compiler lint exemption.
 */
class CollectionStore {
  private listeners = new Set<() => void>();
  private snapshot: Snapshot = { items: [], total: 0 };
  private instance: Virtualizer<ScrollElement, HTMLDivElement>;

  constructor(options: Options) {
    this.instance = new Virtualizer(this.withObservers(options));
  }

  private withObservers(
    options: Options,
  ): VirtualizerOptions<ScrollElement, HTMLDivElement> {
    return {
      ...options,
      scrollToFn: elementScroll,
      observeElementRect,
      observeElementOffset,
      onChange: () => this.publish(),
    };
  }

  private publish = (): void => {
    const items = this.instance.getVirtualItems();
    const total = this.instance.getTotalSize();
    const previous = this.snapshot;
    if (
      total === previous.total &&
      items.length === previous.items.length &&
      items.every((item, index) => {
        const old = previous.items[index];
        return (
          old?.key === item.key &&
          old.index === item.index &&
          old.start === item.start &&
          old.size === item.size &&
          old.end === item.end
        );
      })
    )
      return;
    this.snapshot = { total, items: items.map((item) => ({ ...item })) };
    this.listeners.forEach((listener) => listener());
  };

  getSnapshot = (): Snapshot => this.snapshot;
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  mount = (): (() => void) => this.instance._didMount();

  update(options: Options): void {
    this.instance.setOptions(this.withObservers(options));
    // TanStack's adapter runs this after every commit, when parent refs exist.
    this.instance._willUpdate();
    this.publish();
  }

  measure = (): void => {
    this.instance.measure();
    // Clearing a cache can happen after mounted refs measured their rows. The
    // rows may not get another ResizeObserver event, so restore actual sizes.
    this.instance.getTotalSize();
    this.instance.elementsCache.forEach((element) => {
      const index = this.instance.indexFromElement(element);
      this.instance.resizeItem(
        index,
        this.instance.options.measureElement(element, undefined, this.instance),
      );
    });
    this.publish();
  };
  measureElement = (element: HTMLDivElement | null): void => {
    this.instance.measureElement(element);
    if (element?.isConnected) {
      // The core's normal ref measurement may run before measurementsCache is
      // built for a newly committed layout, or skip while scrolling. Build the
      // layout and synchronously size this visible row instead of leaving its
      // position based on estimates until a later ResizeObserver delivery.
      this.instance.getTotalSize();
      this.instance.resizeItem(
        this.instance.indexFromElement(element),
        this.instance.options.measureElement(element, undefined, this.instance),
      );
    }
  };
  scrollToIndex = (index: number): void =>
    this.instance.scrollToIndex(index, { align: "auto", behavior: "auto" });
}

export function useVirtualCollection(options: Options) {
  const [store] = useState(() => new CollectionStore(options));
  const snapshot = useSyncExternalStore(
    store.subscribe,
    store.getSnapshot,
    store.getSnapshot,
  );
  useLayoutEffect(() => store.mount(), [store]);
  useLayoutEffect(() => {
    store.update(options);
  });
  return {
    ...snapshot,
    measure: store.measure,
    measureElement: store.measureElement,
    scrollToIndex: store.scrollToIndex,
  };
}
