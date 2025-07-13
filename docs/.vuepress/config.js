import { defaultTheme } from '@vuepress/theme-default'
import { defineUserConfig } from 'vuepress'
import { webpackBundler } from '@vuepress/bundler-webpack'

export default defineUserConfig({
  lang: 'zh-CN',

  title: 'Potato',
  description: '极简、高性能的HTTP1.1开发框架',

  theme: defaultTheme({
    //logo: 'https://vuejs.press/images/hero.png',

    navbar: ['/', '/guide/01_hello_world'],
    sidebar: {
      '/guide/': [
        "00_introduction", "01_hello_world", "02_method_annotation", "03_method_declare",
        "04_server_route", "05_graceful_shutdown", "06_client"
      ]
    }
  }),

  bundler: webpackBundler(),
})
