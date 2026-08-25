// useModals — 所有模态窗口开关状态
import { useState } from 'react'

export function useModals() {
  const [showWorkflow, setShowWorkflow] = useState(false)
  const [showMemories, setShowMemories] = useState(false)
  const [showSkills, setShowSkills] = useState(false)
  const [showKnowledge, setShowKnowledge] = useState(false)
  const [showThemes, setShowThemes] = useState(false)
  const [showProject, setShowProject] = useState(false)
  const [showSecurity, setShowSecurity] = useState(false)
  const [showBrowser, setShowBrowser] = useState(false)
  const [showSoul, setShowSoul] = useState(false)
  const [showModels, setShowModels] = useState(false)
  const [showMcp, setShowMcp] = useState(false)
  const [showHelp, setShowHelp] = useState(false)
  const [showSnakeGame, setShowSnakeGame] = useState(false)
  const [showMobile, setShowMobile] = useState(false)
  const [showCustomAgents, setShowCustomAgents] = useState(false)
  const [showExternalAgents, setShowExternalAgents] = useState(false)
  const [showPlugins, setShowPlugins] = useState(false)
  const [showPluginDev, setShowPluginDev] = useState(false)

  return {
    showWorkflow,
    setShowWorkflow,
    showMemories,
    setShowMemories,
    showSkills,
    setShowSkills,
    showKnowledge,
    setShowKnowledge,
    showThemes,
    setShowThemes,
    showProject,
    setShowProject,
    showSecurity,
    setShowSecurity,
    showBrowser,
    setShowBrowser,
    showSoul,
    setShowSoul,
    showModels,
    setShowModels,
    showMcp,
    setShowMcp,
    showHelp,
    setShowHelp,
    showSnakeGame,
    setShowSnakeGame,
    showMobile,
    setShowMobile,
    showCustomAgents,
    setShowCustomAgents,
    showExternalAgents,
    setShowExternalAgents,
    showPlugins,
    setShowPlugins,
    showPluginDev,
    setShowPluginDev,
  }
}
