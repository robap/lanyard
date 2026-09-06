// The zero app, mounted at `/_/log` and nowhere else.
//
// lanyard's `/_/` stays server-rendered with no script tag, because
// `/oidc/authorize` redirects a real browser to it and Phase 4 committed that
// path to working with JavaScript disabled. This app is the log page alone.

import { App } from "zero";
import LiveLog from "./routes/live-log.ts";

new App()
  .route("/_/log", LiveLog)
  // One page: anything else served from this bundle is still the log, because
  // the binary only ever serves `index.html` at `/_/log`.
  .route("*", LiveLog)
  .run("#app");
