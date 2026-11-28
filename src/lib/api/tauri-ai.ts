import {
  aiCliLoginCancel,
  aiCliLoginStart,
  aiCliLoginStatus,
  aiCliLogout,
  analyzeAll,
  generateSubtitlesAll,
  analyzeAssets,
  checkProvider,
  clearApiKey,
  detectAiCli,
  downloadModel,
  getAiSettings,
  getAssetAi,
  giveAiConsent,
  listAiProviders,
  listAiUsage,
  listAnalysisHistory,
  listLocalModels,
  onModelsProgress,
  openAiCliLoginUrl,
  retryFailedAnalysis,
  revokeAiConsent,
  setApiKey,
  setProviderAuthMode,
  setProviderModel,
  updateAiSettings,
  updateAnalysis,
} from "@/lib/ipc"
import type { AiApi } from "@/lib/ipc-contract-ai"

type Call = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>
type Guard = <T>(p: Promise<T>) => Promise<T>

/**
 * P3 AI command 구현 — `ipc.ts` 래퍼로 위임하고 reject 는 `CommandError` 로 정규화한다.
 * `find_similar` 만 아직 래퍼가 없어 직접 invoke 한다. API 키는 인자로만 전달하고 어디에도 저장하지 않는다.
 */
export function createTauriAi(call: Call, guard: Guard): AiApi {
  return {
    getAiSettings: () => guard(getAiSettings()),
    updateAiSettings: (patch) => guard(updateAiSettings(patch)),
    giveAiConsent: () => guard(giveAiConsent()),
    revokeAiConsent: () => guard(revokeAiConsent()),

    listAiProviders: () => guard(listAiProviders()),
    setApiKey: (provider, key, persist) => guard(setApiKey(provider, key, persist)),
    clearApiKey: (provider) => guard(clearApiKey(provider)),
    setProviderModel: (provider, model) => guard(setProviderModel(provider, model)),
    checkProvider: (provider) => guard(checkProvider(provider)),
    setProviderAuthMode: (provider, mode) => guard(setProviderAuthMode(provider, mode)),
    detectAiCli: (provider) => guard(detectAiCli(provider)),
    aiCliLoginStart: (provider) => guard(aiCliLoginStart(provider)),
    aiCliLoginStatus: (provider) => guard(aiCliLoginStatus(provider)),
    aiCliLoginCancel: (provider) => guard(aiCliLoginCancel(provider)),
    openAiCliLoginUrl: (provider) => guard(openAiCliLoginUrl(provider)),
    aiCliLogout: (provider) => guard(aiCliLogout(provider)),

    listLocalModels: () => guard(listLocalModels()),
    downloadModel: (modelId) => guard(downloadModel(modelId)),
    onModelsProgress: (handler) => onModelsProgress(handler),

    analyzeAssets: (assetIds, kinds, force) => guard(analyzeAssets(assetIds, kinds, force)),
    analyzeAll: (onlyUnanalyzed) => guard(analyzeAll(onlyUnanalyzed)),
    generateSubtitlesAll: () => guard(generateSubtitlesAll()),
    retryFailedAnalysis: (assetIds) => guard(retryFailedAnalysis(assetIds)),
    getAssetAi: (assetId) => guard(getAssetAi(assetId)),
    updateAnalysis: (assetId, edit) => guard(updateAnalysis(assetId, edit)),
    listAnalysisHistory: (assetId) => guard(listAnalysisHistory(assetId)),
    listAiUsage: (fromDay, toDay) => guard(listAiUsage(fromDay, toDay)),

    findSimilar: (req) => call("find_similar", { req }),
  }
}
