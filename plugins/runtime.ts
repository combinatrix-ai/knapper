// Headless initialization scheduling only; no elapsed-time timers or host I/O.
(globalThis as any).setTimeout = (callback: () => void) => { Promise.resolve().then(callback); return 0; };
(globalThis as any).clearTimeout = () => {};
(globalThis as any).console = { log() {}, warn() {}, error() {}, debug() {}, info() {}, trace() {} };
