// useModals — 所有模态窗口开关状态
import { useCallback, useState } from 'react'

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
  const [showUpdate, setShowUpdate] = useState(false)
  const [showCanvas, setShowCanvas] = useState(false)
  // 画布工作台的「目标工作流」：由列表入口带过来；为空时工作台自行挑选最近更新的草稿
  const [canvasWorkflowId, setCanvasWorkflowId] = useState<string | null>(null)

  /** 打开画布工作台：传 id 直接编辑该工作流，不传则由工作台自选 */
  const openCanvas = useCallback((id?: string | null) => {
    setCanvasWorkflowId(id ?? null)
    setShowCanvas(true)
  }, [])

  /** 关闭画布工作台并清空目标工作流（避免下次无参打开时沿用旧目标） */
  const closeCanvas = useCallback(() => {
    setShowCanvas(false)
    setCanvasWorkflowId(null)
  }, [])

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
    showUpdate,
    setShowUpdate,
    showCanvas,
    setShowCanvas,
    canvasWorkflowId,
    openCanvas,
    closeCanvas,
  }
}
