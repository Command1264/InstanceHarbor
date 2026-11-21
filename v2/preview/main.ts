import { createApp } from 'vue'
import App from './App.vue'
import { startPreview } from './controller.js'
import '../../docs/design/v2-wireframe/style.css'
import './preview.css'

createApp(App).mount('#preview')
startPreview()
