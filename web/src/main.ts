import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { i18n } from './i18n/i18n.svelte';
import { sendHit } from './lib/hit';

const target = document.getElementById('app')!;
// /admin (contracts/admin.md § 5) : page chargée à la demande (chunk séparé), aucun lien public
if (location.pathname === '/admin') void import('./admin/Admin.svelte').then((m) => mount(m.default, { target }));
else {
  mount(App, { target });
  void sendHit(i18n.lang);
}
