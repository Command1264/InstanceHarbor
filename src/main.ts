import { createApp } from 'vue'
import App from './App.vue'
import { invoke } from '@tauri-apps/api/core'
import { nativeBridge } from './manager'
import { startupMark } from './startup-timing'
// Module bodies run after every import is evaluated, so this is "bundle evaluated".
startupMark('page-navigation-start', performance.timeOrigin)
startupMark('page-script-start')
function report(event:'unhandled-error'|'unhandled-rejection',error?:unknown,line?:number,column?:number){
  const kind=error instanceof Error?error.name:'Unknown'
  if(nativeBridge)void invoke('report_gui_event',{event,kind,line,column}).catch(error=>console.error('Could not write the GUI diagnostic log',error))
}
window.addEventListener('error',event=>report('unhandled-error',event.error,event.lineno,event.colno))
window.addEventListener('unhandledrejection',event=>report('unhandled-rejection',event.reason))
const app=createApp(App)
app.config.errorHandler=(error)=>{console.error(error);report('unhandled-error',error)}
app.mount('#app')
startupMark('page-mounted')
