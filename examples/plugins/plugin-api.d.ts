// MAM 플러그인 API v1 (apiVersion 1) 타입 선언.
// 플러그인 스크립트에서 `/// <reference path="…/plugin-api.d.ts" />` 로 참조한다.
//
// 실행 환경: 호스트가 띄운 별도 프로세스의 QuickJS(ES2020+) — 모듈(import/require)·Node API·네트워크·파일·타이머 없음.
// 전역은 동결된 `mam` 하나뿐이다. 핸들러는 동기 함수여야 한다(Promise 를 돌려주면 호출이 실패한다).
// 한도(제안값): 힙 64 MiB, 스택 512 KiB, 호출당 CPU 2 s, 호출 1회 10 s, 호출당 메시지(host.call·log) 10,000,
// 워커 출력 줄 4 MiB, 소스 main.js 2 MiB. 10분 안에 결함 3회면 플러그인이 자동 비활성된다.

/** 호스트가 거절·실패시킬 때 `mam.host.call` 이 던지는 오류의 `code`. 메시지는 고정 문자열이며 입력을 되돌려주지 않는다 */
type MamHostErrorCode =
  | 'permission_denied' // 매니페스트에 선언하고 사용자가 허용한 권한이 아니거나 플러그인이 비활성
  | 'bad_request' // 알 수 없는 파라미터, 잘못된 id·키·태그 이름·limit (`pluginId` 같은 추가 필드 포함)
  | 'unknown_method' // 아래 `MamHostMethods` 에 없는 메서드
  | 'not_found' // 자산이 없거나, 비활성이거나, 이 호출의 범위(선택·작업 자산) 밖 — 모두 같은 응답
  | 'too_large' // KV 값 64 KiB 초과, 썸네일 1 MiB 초과
  | 'quota_exceeded' // KV 총량 256 MiB, 자산당 태그 100개, 호출당 새 태그 200개, 호출당 파생 데이터 32 MiB
  | 'unavailable' // 썸네일이 아직 없음(재시도 가능)
  | 'busy' // DB 대기 마감 도달(재시도 가능)
  | 'internal' // 호스트 내부 오류 — 상세는 앱 로그에만 남는다
  | 'host_panic';

interface MamHostError extends Error {
  readonly code: MamHostErrorCode;
}

type MamAssetKind = 'image' | 'video' | 'audio';

// ---- 호스트 함수 ----------------------------------------------------------------------------

/** `asset.get` (권한 `asset.read.metadata`). 경로·태그·메모·내용 해시는 주지 않는다. 오프라인 자산도 반환(오류 아님) */
interface MamAssetInfo {
  assetId: number;
  mediaType: MamAssetKind;
  /** `superseded`·`deleted` 도 그대로 돌려준다 */
  status: 'active' | 'superseded' | 'deleted';
  availability: 'online' | 'offline';
  presence: 'present' | 'missing' | null;
  /** 마지막 경로 구성요소(제어문자 제거, 200자) */
  fileName: string | null;
  /** 촬영 시각(ms, epoch) */
  capturedAt: number | null;
  durationMs: number | null;
  width: number | null;
  height: number | null;
}

/** `asset.getDerived` (권한 `asset.read.derived`). v1 은 썸네일만 — 원본 파일은 열지 않는다 */
interface MamDerivedData {
  kind: 'thumbnail';
  size: 'small' | 'large';
  mime: 'image/webp';
  byteLength: number;
  dataBase64: string;
}

interface MamHostMethods {
  'asset.get': { params: { assetId: number }; result: MamAssetInfo };
  'asset.getDerived': { params: { assetId: number; kind: 'thumbnail'; size?: 'small' | 'large' }; result: MamDerivedData };
  /** KV 는 플러그인별 네임스페이스. `assetId` 를 생략하면 플러그인 전역 키. key 1~128자(제어문자 불가) */
  'kv.get': { params: { key: string; assetId?: number | null }; result: { found: false } | { found: true; value: unknown } };
  /** `kv.write`. 값은 JSON 직렬화 65,536 바이트 이하 */
  'kv.set': { params: { key: string; value: unknown; assetId?: number | null }; result: Record<string, never> };
  /** `kv.write`. 없는 키도 오류 아님 */
  'kv.delete': { params: { key: string; assetId?: number | null }; result: { deleted: boolean } };
  /** `kv.read`. 키 이름만 돌려준다. limit 기본 50, 1..=200. `nextCursor` 가 null 이면 끝 */
  'kv.list': {
    params: { assetId?: number | null; prefix?: string; cursor?: string; limit?: number };
    result: { keys: string[]; nextCursor: string | null };
  };
  /** `tag.write`. tags 1~20개, 이름 1~100자. 이미 붙은 태그는 건너뛴다. 제거 API 는 없다 */
  'tag.add': { params: { assetId: number; tags: string[] }; result: { added: number } };
}

// ---- 진입점 ---------------------------------------------------------------------------------

/** 메뉴 명령 호출 인자 — 사용자가 선택한 자산 (중복 제거, 최대 10,000). 이 자산들만 `assetId` 로 접근할 수 있다 */
interface MamCommandParams {
  assetIds: number[];
}

/** 명령 반환값: 문자열이면 사용자에게 토스트로 보인다(제어문자 제거, 200자 절단). 그 밖의 값은 무시 */
type MamCommandResult = string | void | null | undefined;

/** `contributes.providers.metadata|analysis` 호출 인자 — 자산 1개, `revision` 은 처리 대상 자산 revision */
interface MamAutoProviderParams {
  assetId: number;
  kind: 'metadata' | 'analysis';
  revision: number;
}

/** `contributes.providers.export` 호출 인자 — 사용자가 고른 자산(1~5,000) */
interface MamExportProviderParams {
  assetIds: number[];
  kind: 'export';
}

type MamProviderParams = MamAutoProviderParams | MamExportProviderParams;

/**
 * metadata/analysis provider 반환값. 호스트가 검증해 한 트랜잭션으로 기록한다(자산 단위 KV, 태그).
 * `kv` 는 최대 32개 항목(키·값 규칙은 `kv.set` 과 같음, `kv.write` 필요), `tags` 는 1~20개(`tag.write` 필요).
 * 그 밖의 필드·형식 오류는 작업 실패(output_invalid)다.
 */
interface MamAutoProviderOutput {
  kv?: Record<string, unknown>;
  tags?: string[];
}

/** export provider 반환값 — `text` 와 `dataBase64` 중 정확히 하나. 디코딩 후 3 MiB − 4 KiB 이하, 자산 5,000개 이하 */
type MamExportProviderOutput = { text: string } | { dataBase64: string };

type MamProviderResult = MamAutoProviderOutput | MamExportProviderOutput | null | void | undefined;

interface MamLog {
  /** `debug` 는 디버그 로깅일 때만 남는다. 4,096 바이트 초과는 잘림, 제어문자·비밀 형태 문자열은 마스킹, 초당 20건 제한 */
  debug(...args: unknown[]): void;
  info(...args: unknown[]): void;
  warn(...args: unknown[]): void;
  error(...args: unknown[]): void;
}

interface Mam {
  readonly host: {
    /** 호스트 함수를 동기 호출한다. 실패하면 `MamHostError` 를 던진다 */
    call<M extends keyof MamHostMethods>(method: M, params: MamHostMethods[M]['params']): MamHostMethods[M]['result'];
  };
  readonly commands: {
    /** id 는 plugin.json `contributes.commands[].id` 와 같아야 호출된다. 중복 등록은 오류 */
    register(id: string, handler: (params: MamCommandParams) => MamCommandResult): void;
  };
  readonly providers: {
    /** id 는 plugin.json 의 provider id. `params.kind` 로 종류를 구분한다 */
    register(id: string, handler: (params: MamProviderParams) => MamProviderResult): void;
  };
  readonly log: MamLog;
}

declare const mam: Mam;
