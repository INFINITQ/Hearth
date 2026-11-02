// Runs in Hearth's webview before any page script.
/* global window, document */
(() => {
  'use strict';

  // WebRTC can open network connections that the content security policy doesn't cover.
  // Hearth's UI never needs it (all networking happens in Rust), so the constructors go.
  const banned = [
    'RTCPeerConnection',
    'webkitRTCPeerConnection',
    'RTCDataChannel',
    'RTCRtpSender',
    'RTCRtpReceiver',
    'RTCIceCandidate',
    'RTCSessionDescription',
    'WebTransport',
  ];
  for (const name of banned) {
    try {
      Object.defineProperty(window, name, {
        value: undefined,
        writable: false,
        configurable: false,
      });
    } catch {
      // Already gone or locked: nothing to do.
    }
  }

  // No DNS prefetching for links. (index.html also sets this statically.)
  const noPrefetch = () => {
    const meta = document.createElement('meta');
    meta.httpEquiv = 'x-dns-prefetch-control';
    meta.content = 'off';
    (document.head || document.documentElement).prepend(meta);
  };
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', noPrefetch, { once: true });
  } else {
    noPrefetch();
  }
})();
