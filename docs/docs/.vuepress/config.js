import { defaultTheme } from '@vuepress/theme-default'
import { defineUserConfig } from 'vuepress'
import { webpackBundler } from '@vuepress/bundler-webpack'

export default defineUserConfig({
  lang: 'zh-CN',

  title: 'Potato',
  description: '极简、高性能的HTTP1.1开发框架',

  theme: defaultTheme({
    //logo: 'https://vuejs.press/images/hero.png',

    navbar: ['/', '/guide/00_introduction'],
    sidebar: {
      '/guide/': [
        "00_introduction", "01_method_annotation",
        "02_method_declare", "03_server_route",
        "04_graceful_shutdown", "05_client"
      ]
    }
  }),

  bundler: webpackBundler(),
})
