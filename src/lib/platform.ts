/** True when the app runs on Windows. */
export const isWindows = () => typeof navigator !== 'undefined' && /Win/i.test(navigator.platform);
