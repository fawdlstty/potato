import { defaultTheme } from '@vuepress/theme-default'
import { defineUserConfig } from 'vuepress'
import { webpackBundler } from '@vuepress/bundler-webpack'

export default defineUserConfig({
  locales: {
    '/': {
      lang: 'zh-CN',
      title: 'Potato',
      description: '极简、高性能的HTTP开发框架'
    },
    '/en/': {
      lang: 'en-US',
      title: 'Potato',
      description: 'High-performance, concise syntax HTTP framework'
    }
  },

  theme: defaultTheme({
    locales: {
      '/': {
        selectLanguageName: '简体中文',
        selectLanguageText: 'Languages',
        selectLanguageAriaLabel: 'Select language',
        //logo: 'https://vuejs.press/images/hero.png',
        navbar: [
          '/',
          '/guide/01_hello_world',
          {
            text: 'GitHub',
            link: 'https://github.com/fawdlstty/potato'
          }
        ],
        sidebar: {
          '/guide/': [
            "00_introduction", "01_hello_world", "02_method_annotation", "03_method_declare",
            "04_server_route", "05_graceful_shutdown", "06_client"
          ]
        }
      },
      '/en/': {
        selectLanguageName: 'English',
        selectLanguageText: 'Languages',
        selectLanguageAriaLabel: 'Select language',
        //logo: 'https://vuejs.press/images/hero.png',
        navbar: [
          '/en/',
          '/en/guide/01_hello_world.html',
          {
            text: 'GitHub',
            link: 'https://github.com/fawdlstty/potato'
          }
        ],
        sidebar: {
          '/en/guide/': [
            "00_introduction", "01_hello_world", "02_method_annotation", "03_method_declare",
            "04_server_route", "05_graceful_shutdown", "06_client"
          ]
        }
      }
    }
  }),

  bundler: webpackBundler(),
})
