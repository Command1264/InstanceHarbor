// Host replies after which a console tab cannot show its server any more: the run
// ended, or a new run replaced it. The tab then closes; closing the last tab ends
// the viewer (ConsoleViewer::close in console_viewer.rs).
const RUN_GONE = new Set(['ConsoleRunEnded', 'ConsoleRunChanged'])

export function endsTab(error) {
  return typeof error === 'string' && RUN_GONE.has(error)
}
