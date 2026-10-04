import { mount } from 'svelte';
import './app.css';
import './lib/design';
import App from './App.svelte';

export default mount(App, { target: document.getElementById('app')! });
