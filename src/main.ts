import { createApp } from 'vue'
import App from './App.vue'
import './styles.css'
import { installThemes } from './themes'

installThemes()

createApp(App).mount('#app')
