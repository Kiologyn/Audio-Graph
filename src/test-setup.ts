// Interface modules run in a page: what they touch while loading, for tests in Node
globalThis.window ??= globalThis as typeof window;
window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList;
