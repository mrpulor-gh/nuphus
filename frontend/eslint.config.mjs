import tseslint from '@typescript-eslint/eslint-plugin'
import tsparser from '@typescript-eslint/parser'
import reactPlugin from 'eslint-plugin-react'
import reactHooksPlugin from 'eslint-plugin-react-hooks'
import prettier from 'eslint-config-prettier'

export default [
  { ignores: ['dist', 'node_modules', '*.config.*', '*.css'] },
  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      parser: tsparser,
      parserOptions: {
        ecmaVersion: 'latest',
        sourceType: 'module',
        ecmaFeatures: { jsx: true },
      },
    },
    plugins: {
      '@typescript-eslint': tseslint,
      react: reactPlugin,
      'react-hooks': reactHooksPlugin,
    },
    settings: { react: { version: 'detect' } },
    rules: {
      'react/react-in-jsx-scope': 'off',
      'react/prop-types': 'off',
      '@typescript-eslint/no-explicit-any': 'warn',
      '@typescript-eslint/no-unused-vars': ['warn', { argsIgnorePattern: '^_' }],
      'no-console': 'off',
      // 图标统一经 ui/Icons.tsx 出口（IconXxx 命名）：业务组件不要直接依赖图标库，
      // 缺失的图标请在 ui/Icons.tsx 补导出。存量违规保持 warn 级（不阻断 CI），
      // 新代码应为零违规；逐个清理时无需改本配置。
      'no-restricted-imports': [
        'warn',
        {
          paths: [
            {
              name: 'lucide-react',
              message:
                '图标请经 ui/Icons.tsx 出口导入（IconXxx 命名）；缺失的图标在 ui/Icons.tsx 补导出。',
            },
          ],
        },
      ],
      ...prettier.rules,
    },
  },
  {
    // 图标出口自身必须直接依赖图标库
    files: ['src/ui/Icons.tsx'],
    rules: { 'no-restricted-imports': 'off' },
  },
]
