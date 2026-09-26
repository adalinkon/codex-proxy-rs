import type { RouteRecordRaw } from 'vue-router'

export const routes: RouteRecordRaw[] = [
  {
    path: '/login',
    name: 'login',
    component: () => import('@/views/login/index.vue'),
  },
  {
    path: '/',
    component: () => import('@/layout/index.vue'),
    children: [
      { path: 'users', name: 'users', component: () => import('@/views/users/index.vue') },
      { path: 'me/profile', name: 'my-profile', component: () => import('@/views/user/index.vue') },
      { path: 'me/overview', name: 'my-overview', redirect: '/me/profile' },
      { path: 'me/keys', name: 'my-keys', component: () => import('@/views/keys/index.vue'), props: { scope: 'user' } },
      { path: 'me/usage', name: 'my-usage', component: () => import('@/views/usage/index.vue'), props: { scope: 'user' } },
      { path: 'me/settings', name: 'my-settings', redirect: '/me/profile' },
      {
        path: '',
        name: 'dashboard',
        component: () => import('@/views/dashboard/index.vue'),
      },
      {
        path: 'accounts',
        name: 'accounts',
        component: () => import('@/views/accounts/index.vue'),
      },
      {
        path: 'proxies',
        name: 'proxies',
        component: () => import('@/views/proxies/index.vue'),
      },
      {
        path: 'groups',
        name: 'groups',
        component: () => import('@/views/groups/index.vue'),
      },
      {
        path: 'keys',
        name: 'keys',
        component: () => import('@/views/keys/index.vue'),
      },
      {
        path: 'usage',
        name: 'usage',
        component: () => import('@/views/usage/index.vue'),
      },
      {
        path: 'plugins',
        component: () => import('@/views/plugins/index.vue'),
        children: [
          {
            path: '',
            name: 'plugins',
            component: () => import('@/views/plugins/components/PluginManagement.vue'),
          },
          {
            path: ':instanceId/:pageId',
            name: 'plugin-page',
            component: () => import('@/views/plugins/components/PluginPage.vue'),
          },
        ],
      },
      {
        path: 'theme',
        name: 'theme',
        component: () => import('@/views/theme/index.vue'),
      },
      {
        path: 'settings',
        children: [
          {
            path: '',
            name: 'settings',
            component: () => import('@/views/settings/index.vue'),
          },
          {
            path: 'upstream',
            name: 'settings-upstream',
            component: () => import('@/views/settings/index.vue'),
          },
          {
            path: 'access',
            name: 'settings-access',
            component: () => import('@/views/settings/index.vue'),
          },
          {
            path: 'backup',
            name: 'settings-backup',
            component: () => import('@/views/settings/index.vue'),
          },
          {
            path: 'pricing',
            name: 'settings-pricing',
            component: () => import('@/views/settings/index.vue'),
          },
        ],
      },
    ],
  },
  {
    path: '/:pathMatch(.*)*',
    redirect: '/',
  },
]
