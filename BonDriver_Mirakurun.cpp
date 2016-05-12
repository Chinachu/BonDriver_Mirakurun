#include "BonDriver_Mirakurun.h"

#pragma comment(lib, "ws2_32.lib")

#define BITRATE_CALC_TIME	500		//ms

//////////////////////////////////////////////////////////////////////
// 定数定義
//////////////////////////////////////////////////////////////////////

// ミューテックス名
#define MUTEX_NAME			TEXT(TUNER_NAME)

// FIFOバッファ設定
#define ASYNCBUFFTIME		2											// バッファ長 = 2秒
#define ASYNCBUFFSIZE		( 0x200000 / TSDATASIZE * ASYNCBUFFTIME )	// 平均16Mbpsとする

#define REQRESERVNUM		8				// 非同期リクエスト予約数 //before 16
#define REQPOLLINGWAIT		20				// 非同期リクエストポーリング間隔(ms) //before 10

// 非同期リクエスト状態
#define IORS_IDLE			0x00			// リクエスト空
#define IORS_BUSY			0x01			// リクエスト受信中
#define IORS_RECV			0x02			// 受信完了、ストア待ち

static int Init(HMODULE hModule)
{
	GetModuleFileName(hModule, g_IniFilePath, MAX_PATH);

	wchar_t drive[_MAX_DRIVE];
	wchar_t dir[_MAX_DIR];
	wchar_t fname[_MAX_FNAME];
	_wsplitpath_s(g_IniFilePath, drive, sizeof(drive), dir, sizeof(dir), fname, sizeof(fname), NULL, NULL);
	wsprintf(g_IniFilePath, L"%s%s%s.ini\0", drive, dir, fname);

	HANDLE hFile = CreateFile(g_IniFilePath, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
	if (hFile == INVALID_HANDLE_VALUE) {
		return -2;
	}
	CloseHandle(hFile);
	size_t ret;

	wchar_t tmpServerHost[MAX_HOST_LEN];
	GetPrivateProfileString(L"GLOBAL", L"SERVER_HOST", L"localhost", tmpServerHost, sizeof(tmpServerHost), g_IniFilePath);
	wcstombs_s(&ret, g_ServerHost, tmpServerHost, sizeof(g_ServerHost));

	wchar_t tmpServerPort[MAX_PORT_LEN];
	GetPrivateProfileString(L"GLOBAL", L"SERVER_PORT", L"8888", tmpServerPort, sizeof(tmpServerPort), g_IniFilePath);
	wcstombs_s(&ret, g_ServerPort, tmpServerPort, sizeof(g_ServerPort));

	g_DecodeB25 = GetPrivateProfileInt(L"GLOBAL", L"DECODE_B25", 0, g_IniFilePath);
	g_Priority = GetPrivateProfileInt(L"GLOBAL", L"PRIORITY", 0, g_IniFilePath);

	setlocale(LC_ALL, "japanese");

	g_MagicPacket_Enable = GetPrivateProfileInt(L"GLOBAL", L"MAGICPACKET_ENABLE", 0, g_IniFilePath);

	if (g_MagicPacket_Enable) {
		wchar_t tmpMagicPacket_TargetMAC[18];
		GetPrivateProfileString(L"GLOBAL", L"MAGICPACKET_TARGETMAC", L"00:00:00:00:00:00", tmpMagicPacket_TargetMAC, sizeof(tmpMagicPacket_TargetMAC), g_IniFilePath);
		wcstombs_s(&ret, g_MagicPacket_TargetMAC, tmpMagicPacket_TargetMAC, sizeof(g_MagicPacket_TargetMAC));


		for (int i = 0; i < 6; i++) {
			BYTE b = 0;
			char *p = &g_MagicPacket_TargetMAC[i * 3];
			for (int j = 0; j < 2; j++) {
				if (*p >= '0' && *p <= '9') {
					b = b * 0x10 + (*p - '0');
				} else if (*p >= 'a' && *p <= 'f') {
					b = b * 0x10 + (10 + *p - 'a');
				} else if (*p >= 'A' && *p <= 'F') {
					b = b * 0x10 + (10 + *p - 'A');
				}
				p++;
			}
			g_MagicPacket_TargetMAC[i] = b;
		}
		wchar_t tmpMagicPacket_TargetIP[16];
		GetPrivateProfileString(L"GLOBAL", L"MAGICPACKET_TARGETIP", L"0.0.0.0", tmpMagicPacket_TargetIP, sizeof(tmpMagicPacket_TargetIP), g_IniFilePath);
		wcstombs_s(&ret, g_MagicPacket_TargetIP, tmpMagicPacket_TargetIP, sizeof(g_MagicPacket_TargetIP));

	}

	return 0;
}

BOOL APIENTRY DllMain(HINSTANCE hModule, DWORD fdwReason, LPVOID lpReserved)
{
	switch (fdwReason) {
		case DLL_PROCESS_ATTACH:
			if (Init(hModule) != 0) {
				return FALSE;
			}
			// モジュールハンドル保存
			CBonTuner::m_hModule = hModule;
			break;

		case DLL_PROCESS_DETACH:
			// 未開放の場合はインスタンス開放
			if (CBonTuner::m_pThis) {
				CBonTuner::m_pThis->Release();
			}
			break;
	}

	return TRUE;
}

inline DWORD DiffTime(DWORD BeginTime,DWORD EndTime)
{
	if (BeginTime <= EndTime)
		return EndTime-BeginTime;
	return (0xFFFFFFFFUL-BeginTime)+EndTime+1UL;
}


//////////////////////////////////////////////////////////////////////
// インスタンス生成メソッド
//////////////////////////////////////////////////////////////////////

extern "C" __declspec(dllexport) IBonDriver * CreateBonDriver()
{
	// スタンス生成(既存の場合はインスタンスのポインタを返す)
	return (CBonTuner::m_pThis)? CBonTuner::m_pThis : ((IBonDriver *) new CBonTuner);
}


//////////////////////////////////////////////////////////////////////
// 構築/消滅
//////////////////////////////////////////////////////////////////////

// 静的メンバ初期化
CBonTuner * CBonTuner::m_pThis = NULL;
HINSTANCE CBonTuner::m_hModule = NULL;

CBonTuner::CBonTuner()
	: m_bTunerOpen(false)
	, m_hMutex(NULL)
	, m_pIoReqBuff(NULL)
	, m_pIoPushReq(NULL)
	, m_pIoPopReq(NULL)
	, m_pIoGetReq(NULL)
	, m_dwBusyReqNum(0UL)
	, m_dwReadyReqNum(0UL)
	, m_hPushIoThread(NULL)
	, m_hPopIoThread(NULL)
	, m_hOnStreamEvent(NULL)
	, m_dwCurSpace(0UL)
	, m_dwCurChannel(0xFFFFFFFFUL)
	, m_sock(INVALID_SOCKET)
	, m_fBitRate(0.0f)
	, m_dwRecvBytes(0UL)
	, m_dwLastCalcTick(0UL)
{
	m_pThis = this;

	// クリティカルセクション初期化
	::InitializeCriticalSection(&m_CriticalSection);
}

CBonTuner::~CBonTuner()
{
	// 開かれてる場合は閉じる
	CloseTuner();

	// クリティカルセクション削除
	::DeleteCriticalSection(&m_CriticalSection);

	// Winsock終了
	if (m_bTunerOpen) {
		WSACleanup();
	}

	m_pThis = NULL;
}

const BOOL CBonTuner::OpenTuner()
{
	if (!m_bTunerOpen) {
		// Winsock初期化
		WSADATA stWsa;
		if (WSAStartup(MAKEWORD(2,2), &stWsa) != 0) {
			return FALSE;
		}
		if (g_MagicPacket_Enable) {
			char magicpacket[102];
			memset(&magicpacket, 0xff, 6);
			for (int i = 0; i < 16; i++) {
				memcpy(&magicpacket[i * 6 + 6], g_MagicPacket_TargetMAC, 6);
			}
			SOCKET s = socket(PF_INET, SOCK_DGRAM, 0);
			if (s == SOCKET_ERROR) {
				return FALSE;
			}
			SOCKADDR_IN addr;
			addr.sin_family = AF_INET;
			addr.sin_port = htons(9);
			addr.sin_addr.S_un.S_addr = inet_addr(g_MagicPacket_TargetIP);

			sendto(s, magicpacket, sizeof(magicpacket), 0, (LPSOCKADDR)&addr, sizeof(addr));
		}

		m_bTunerOpen = true;

		// Mirakurun APIよりchannel取得
		GetApiChannels("GR", &g_Channel_JSON_GR);
		GetApiChannels("BS", &g_Channel_JSON_BS);
		GetApiChannels("CS", &g_Channel_JSON_CS);
	}


	//return SetChannel(0UL,0UL);
	
	return TRUE;
}

void CBonTuner::CloseTuner()
{
	// スレッド終了要求セット
	m_bLoopIoThread = FALSE;

	// イベント開放
	if (m_hOnStreamEvent) {
		::CloseHandle(m_hOnStreamEvent);
		m_hOnStreamEvent = NULL;
	}

	// スレッド終了
	if (m_hPushIoThread) {
		if (::WaitForSingleObject(m_hPushIoThread, 1000) != WAIT_OBJECT_0) {
			// スレッド強制終了
			::TerminateThread(m_hPushIoThread, 0);

			TCHAR szDebugOut[128];
			::wsprintf(szDebugOut, TEXT("%s: CBonTuner::CloseTuner() ::TerminateThread(m_hPushIoThread)\n"), TUNER_NAME);
			::OutputDebugString(szDebugOut);
		}

		::CloseHandle(m_hPushIoThread);
		m_hPushIoThread = NULL;
	}

	if (m_hPopIoThread) {
		if (::WaitForSingleObject(m_hPopIoThread, 1000) != WAIT_OBJECT_0) {
			// スレッド強制終了
			::TerminateThread(m_hPopIoThread, 0);

			TCHAR szDebugOut[128];
			::wsprintf(szDebugOut, TEXT("%s: CBonTuner::CloseTuner() ::TerminateThread(m_hPopIoThread)\n"), TUNER_NAME);
			::OutputDebugString(szDebugOut);
		}

		::CloseHandle(m_hPopIoThread);
		m_hPopIoThread = NULL;
	}


	// バッファ開放
	FreeIoReqBuff(m_pIoReqBuff);
	m_pIoReqBuff = NULL;
	m_pIoPushReq = NULL;
	m_pIoPopReq = NULL;
	m_pIoGetReq = NULL;

	m_dwBusyReqNum = 0UL;
	m_dwReadyReqNum = 0UL;

	// ソケットクローズ
	if (m_sock != INVALID_SOCKET) {
		closesocket(m_sock);
		m_sock = INVALID_SOCKET;
	}

	// チャンネル初期化
	m_dwCurSpace = 0UL;
	m_dwCurChannel = 0xFFFFFFFFUL;

	// ミューテックス開放
	if (m_hMutex) {
		::ReleaseMutex(m_hMutex);
		::CloseHandle(m_hMutex);
		m_hMutex = NULL;
	}

	m_fBitRate = 0.0f;
	m_dwRecvBytes = 0UL;
}

const DWORD CBonTuner::WaitTsStream(const DWORD dwTimeOut)
{
	// 終了チェック
	if (!m_hOnStreamEvent || !m_bLoopIoThread) {
		return WAIT_ABANDONED;
	}

	// イベントがシグナル状態になるのを待つ
	const DWORD dwRet = ::WaitForSingleObject(m_hOnStreamEvent, (dwTimeOut)? dwTimeOut : INFINITE);

	switch (dwRet) {
		case WAIT_ABANDONED :
			// チューナが閉じられた
			return WAIT_ABANDONED;

		case WAIT_OBJECT_0 :
		case WAIT_TIMEOUT :
			// ストリーム取得可能 or チューナが閉じられた
			return (m_bLoopIoThread)? dwRet : WAIT_ABANDONED;

		case WAIT_FAILED :
		default:
			// 例外
			return WAIT_FAILED;
	}
}

const DWORD CBonTuner::GetReadyCount()
{
	// 取り出し可能TSデータ数を取得する
	return m_dwReadyReqNum;
}

const BOOL CBonTuner::GetTsStream(BYTE *pDst, DWORD *pdwSize, DWORD *pdwRemain)
{
	BYTE *pSrc = NULL;

	// TSデータをバッファから取り出す
	if (GetTsStream(&pSrc, pdwSize, pdwRemain)) {
		if (*pdwSize) {
			::CopyMemory(pDst, pSrc, *pdwSize);
		}

		return TRUE;
	}

	return FALSE;
}

const BOOL CBonTuner::GetTsStream(BYTE **ppDst, DWORD *pdwSize, DWORD *pdwRemain)
{
	if (!m_pIoGetReq) {
		return FALSE;
	}

	// TSデータをバッファから取り出す
	if (m_dwReadyReqNum) {
		if (m_pIoGetReq->dwState == IORS_RECV) {

			// データコピー
			*pdwSize = m_pIoGetReq->dwRxdSize;
			*ppDst = m_pIoGetReq->RxdBuff;

			// バッファ位置を進める
			::EnterCriticalSection(&m_CriticalSection);
			m_pIoGetReq = m_pIoGetReq->pNext;
			m_dwReadyReqNum--;
			*pdwRemain = m_dwReadyReqNum;
			::LeaveCriticalSection(&m_CriticalSection);

			return TRUE;
		}

		// 例外
		return FALSE;
	}

	// 取り出し可能なデータがない
	*pdwSize = 0;
	*pdwRemain = 0;

	return TRUE;
}

void CBonTuner::PurgeTsStream()
{
	// バッファから取り出し可能データをパージする

	::EnterCriticalSection(&m_CriticalSection);
	m_pIoGetReq = m_pIoPopReq;
	m_dwReadyReqNum = 0;
	::LeaveCriticalSection(&m_CriticalSection);
}

void CBonTuner::Release()
{
	// インスタンス開放
	delete this;
}

LPCTSTR CBonTuner::GetTunerName(void)
{
	// チューナ名を返す
	return TEXT(TUNER_NAME);
}

const BOOL CBonTuner::IsTunerOpening(void)
{
	// チューナの使用中の有無を返す(全プロセスを通して)
	HANDLE hMutex = ::OpenMutex(MUTEX_ALL_ACCESS, FALSE, MUTEX_NAME);

	if (hMutex) {
		// 既にチューナは開かれている
		::CloseHandle(hMutex);
		return TRUE;
	}

	// チューナは開かれていない
	return FALSE;
}

LPCTSTR CBonTuner::EnumTuningSpace(const DWORD dwSpace)
{
	// 使用可能なチューニング空間を返す
	switch (dwSpace) {
		case 0UL :
			return TEXT("GR");
		case 1UL :
			return TEXT("BS");
		case 2UL:
			return TEXT("CS");
		default  :
			return NULL;
	}
}

LPCTSTR CBonTuner::EnumChannelName(const DWORD dwSpace, const DWORD dwChannel)
{
	picojson::value channel_json;
	LPCTSTR space_str = CBonTuner::EnumTuningSpace(dwSpace);

	if (space_str == L"GR") {
		channel_json = g_Channel_JSON_GR;
	} else if (space_str == L"BS") {
		channel_json = g_Channel_JSON_BS;
	} else if (space_str == L"CS") {
		channel_json = g_Channel_JSON_CS;
	} else {
		return NULL;
	}

	if (channel_json.is<picojson::array>() == false
		|| channel_json.get<picojson::array>().empty()
		|| channel_json.contains(dwChannel) == false) {
		return NULL;
	}

	picojson::object& channel_obj = channel_json.get(dwChannel).get<picojson::object>();
	std::string channel_name = channel_obj["name"].get<std::string>();

	static TCHAR buf[128];
	mbstowcs(buf, channel_name.c_str(), sizeof(buf));

	return buf;
}

const DWORD CBonTuner::GetCurSpace(void)
{
	// 現在のチューニング空間を返す
	return m_dwCurSpace;
}

const DWORD CBonTuner::GetCurChannel(void)
{
	// 現在のチャンネルを返す
	return m_dwCurChannel;
}

CBonTuner::AsyncIoReq * CBonTuner::AllocIoReqBuff(const DWORD dwBuffNum)
{
	if (dwBuffNum < 2) {
		return NULL;
	}

	// メモリを確保する
	AsyncIoReq *pNewBuff = new AsyncIoReq [dwBuffNum];
	if (!pNewBuff) {
		return NULL;
	}

	// ゼロクリア
	::ZeroMemory(pNewBuff, sizeof(AsyncIoReq) * dwBuffNum);

	// リンクを構築する
	DWORD dwIndex;
	for(dwIndex = 0 ; dwIndex < ( dwBuffNum - 1 ) ; dwIndex++) {
		pNewBuff[dwIndex].pNext= &pNewBuff[dwIndex + 1];
	}

	pNewBuff[dwIndex].pNext = &pNewBuff[0];

	return pNewBuff;
}

void CBonTuner::FreeIoReqBuff(CBonTuner::AsyncIoReq *pBuff)
{
	if (!pBuff) {
		return;
	}

	// バッファを開放する
	delete [] pBuff;
}

DWORD WINAPI CBonTuner::PushIoThread(LPVOID pParam)
{
	CBonTuner *pThis = (CBonTuner *)pParam;

	DWORD dwLastTime = ::GetTickCount();

//	::OutputDebugString("CBonTuner::PushIoThread() Start!\n");

	// ドライバにTSデータリクエストを発行する
	while (pThis->m_bLoopIoThread) {

		// リクエスト処理待ちが規定未満なら追加する
		if (pThis->m_dwBusyReqNum < REQRESERVNUM) {

			// ドライバにTSデータリクエストを発行する(HTTPなので受信要求のみ)
			if (!pThis->PushIoRequest(pThis->m_sock)) {
				// エラー発生
				break;
			}

		} else {
			// リクエスト処理待ちがフルの場合はウェイト
			::Sleep(REQPOLLINGWAIT);
		}
	}

//	::OutputDebugString("CBonTuner::PushIoThread() End!\n");
	return 0;
}

DWORD WINAPI CBonTuner::PopIoThread(LPVOID pParam)
{
	CBonTuner *pThis = (CBonTuner *)pParam;

	// 処理済リクエストをポーリングしてリクエストを完了させる
	while (pThis->m_bLoopIoThread) {

		// 処理済データがあればリクエストを完了する
		if (pThis->m_dwBusyReqNum) {

			// リクエストを完了する
			if (!pThis->PopIoRequest(pThis->m_sock)) {
				// エラー発生
				break;
			}
		}
	}

	return 0;
}

const BOOL CBonTuner::PushIoRequest(SOCKET sock)
{
	// ドライバに非同期リクエストを発行する

	// オープンチェック
	if (sock == INVALID_SOCKET)return FALSE;

	// リクエストセット
	m_pIoPushReq->dwRxdSize = 0;

	// イベント設定
	::ZeroMemory(&m_pIoPushReq->OverLapped, sizeof(WSAOVERLAPPED));
	if (!(m_pIoPushReq->OverLapped.hEvent = ::CreateEvent(NULL, TRUE, FALSE, NULL)))return FALSE;

	// HTTP受信を要求スルニダ！
	DWORD Flags = 0;
	WSABUF wsaBuf;
	wsaBuf.buf = (char*)m_pIoPushReq->RxdBuff;
	wsaBuf.len = sizeof(m_pIoPushReq->RxdBuff);
	if (SOCKET_ERROR == WSARecv(sock, &wsaBuf, 1, &m_pIoPushReq->dwRxdSize, &Flags, &m_pIoPushReq->OverLapped, NULL)) {
		int sock_err = WSAGetLastError();
		if (sock_err != ERROR_IO_PENDING) {
			return FALSE;
		}
	}

	m_pIoPushReq->dwState = IORS_BUSY;

	// バッファ状態更新
	::EnterCriticalSection(&m_CriticalSection);
	m_pIoPushReq = m_pIoPushReq->pNext;
	m_dwBusyReqNum++;
	::LeaveCriticalSection(&m_CriticalSection);

	return TRUE;
}

const BOOL CBonTuner::PopIoRequest(SOCKET sock)
{
	// 非同期リクエストを完了する

	// オープンチェック
	if (sock == INVALID_SOCKET) {
		return FALSE;
	}

	// 状態チェック
	if (m_pIoPopReq->dwState != IORS_BUSY) {
		// 例外
		return TRUE;
	}

	// リクエスト取得
	DWORD Flags=0;
	const BOOL bRet = ::WSAGetOverlappedResult(sock, &m_pIoPopReq->OverLapped, &m_pIoPopReq->dwRxdSize, FALSE, &Flags);

	// エラーチェック
	if (!bRet) {
		int sock_err = WSAGetLastError();
		if (sock_err == ERROR_IO_INCOMPLETE) {
			// 処理未完了
			::Sleep(REQPOLLINGWAIT);
			return TRUE;
		}
	}

	// 総受信サイズ加算
	m_dwRecvBytes += m_pIoPopReq->dwRxdSize;

	// ビットレート計算
	if (DiffTime(m_dwLastCalcTick,::GetTickCount()) >= BITRATE_CALC_TIME) {
		CalcBitRate();
	}

	// イベント削除
	::CloseHandle(m_pIoPopReq->OverLapped.hEvent);

	if (!bRet) {
		// エラー発生
		return FALSE;
	}

	m_pIoPopReq->dwState = IORS_RECV;

	// バッファ状態更新
	::EnterCriticalSection(&m_CriticalSection);
	m_pIoPopReq = m_pIoPopReq->pNext;
	m_dwBusyReqNum--;
	m_dwReadyReqNum++;
	::LeaveCriticalSection(&m_CriticalSection);

	// イベントセット
	::SetEvent(m_hOnStreamEvent);

	return TRUE;
}


// チャンネル設定
const BOOL CBonTuner::SetChannel(const BYTE bCh)
{
	return SetChannel((DWORD)0,(DWORD)bCh - 13);
}

// チャンネル設定
const BOOL CBonTuner::SetChannel(const DWORD dwSpace, const DWORD dwChannel)
{
	picojson::value channel_json;
	LPCTSTR space_str = CBonTuner::EnumTuningSpace(dwSpace);

	if (space_str == L"GR") {
		channel_json = g_Channel_JSON_GR;
	}
	else if (space_str == L"BS") {
		channel_json = g_Channel_JSON_BS;
	}
	else if (space_str == L"CS") {
		channel_json = g_Channel_JSON_CS;
	}
	else {
		return NULL;
	}

	if (channel_json.is<picojson::array>() == false
		|| channel_json.get<picojson::array>().empty()
		|| channel_json.contains(dwChannel) == false) {
		return NULL;
	}

	picojson::object& channel_obj = channel_json.get(dwChannel).get<picojson::object>();
	std::string channel_name = channel_obj["name"].get<std::string>();
	std::string channel = channel_obj["channel"].get<std::string>();

	wchar_t wChannel[16];
	mbstowcs(wChannel, channel.c_str(), sizeof(wChannel));

	// 一旦クローズ
	CloseTuner();

	// バッファ確保
	if (!(m_pIoReqBuff = AllocIoReqBuff(ASYNCBUFFSIZE))) {
		return FALSE;
	}

	// バッファ位置同期
	m_pIoPushReq = m_pIoReqBuff;
	m_pIoPopReq = m_pIoReqBuff;
	m_pIoGetReq = m_pIoReqBuff;
	m_dwBusyReqNum = 0;
	m_dwReadyReqNum = 0;

	try{
		char serverRequest[256];

		// tmp
		wchar_t tmpString[128];
		wchar_t tmpUrl[128];

		WCHAR tmpServerRequest[256];

		// URL生成
		wsprintf(tmpUrl, L"/api/channels/%s/%s/stream?decode=%d", CBonTuner::EnumTuningSpace(dwSpace), wChannel, g_DecodeB25);

		wsprintf(tmpServerRequest, L"GET %s HTTP/1.0\r\nX-Mirakurun-Priority: %d\r\n\r\n", tmpUrl, g_Priority);

		size_t i;
		wcstombs_s(&i, serverRequest, tmpServerRequest, sizeof(serverRequest));

		struct addrinfo hints;
		struct addrinfo* res = NULL;
		struct addrinfo* ai;

		memset(&hints, 0, sizeof(hints));
		hints.ai_family = AF_INET6;	//IPv6優先
		hints.ai_socktype = SOCK_STREAM;
		hints.ai_protocol = IPPROTO_TCP;
		hints.ai_flags = AI_NUMERICSERV;
		if (getaddrinfo(g_ServerHost, g_ServerPort, &hints, &res) != 0) {
			//printf("getaddrinfo(): %s\n", gai_strerror(err));
			hints.ai_family = AF_INET;	//IPv4限定
			if (getaddrinfo(g_ServerHost, g_ServerPort, &hints, &res) != 0) {
				throw 1UL;
			}
		}

		for (ai = res; ai; ai = ai->ai_next) {
			m_sock = socket(ai->ai_family, ai->ai_socktype, ai->ai_protocol);
			if (m_sock == INVALID_SOCKET) {
				continue;
			}

			if (connect(m_sock, ai->ai_addr, ai->ai_addrlen) >= 0) {
				// OK
				break;
			}
			closesocket(m_sock);
			m_sock = INVALID_SOCKET;
		}
		freeaddrinfo(res);

		if (m_sock == INVALID_SOCKET) {
			TCHAR szDebugOut[128];
			::wsprintf(szDebugOut, TEXT("%s: CBonTuner::OpenTuner() connection error %d\n"), TUNER_NAME, WSAGetLastError());
			::OutputDebugString(szDebugOut);
			throw 1UL;
		}

		if (send(m_sock, serverRequest, (int)strlen(serverRequest), 0) < 0) {
			TCHAR szDebugOut[128];
			::wsprintf(szDebugOut, TEXT("%s: CBonTuner::OpenTuner() send error %d\n"), TUNER_NAME, WSAGetLastError());
			::OutputDebugString(szDebugOut);
			throw 1UL;
		}

		// イベント作成
		if (!(m_hOnStreamEvent = ::CreateEvent(NULL, FALSE, FALSE, NULL))) {
			throw 2UL;
		}

		// スレッド起動
		DWORD dwPushIoThreadID = 0UL, dwPopIoThreadID = 0UL;
		m_hPushIoThread = ::CreateThread(NULL, 0UL, CBonTuner::PushIoThread, this, CREATE_SUSPENDED, &dwPopIoThreadID);
		m_hPopIoThread = ::CreateThread(NULL, 0UL, CBonTuner::PopIoThread, this, CREATE_SUSPENDED, &dwPushIoThreadID);

		if (!m_hPushIoThread || !m_hPopIoThread) {
			if (m_hPushIoThread) {
				::TerminateThread(m_hPushIoThread, 0UL);
				::CloseHandle(m_hPushIoThread);
				m_hPushIoThread = NULL;
			}

			if (m_hPopIoThread) {
				::TerminateThread(m_hPopIoThread, 0UL);
				::CloseHandle(m_hPopIoThread);
				m_hPopIoThread = NULL;
			}

			throw 3UL;
		}

		// スレッド開始
		m_bLoopIoThread = TRUE;
		if (::ResumeThread(m_hPushIoThread) == 0xFFFFFFFFUL || ::ResumeThread(m_hPopIoThread) == 0xFFFFFFFFUL) {
			throw 4UL;
		}

		// ミューテックス作成
		if (!(m_hMutex = ::CreateMutex(NULL, TRUE, MUTEX_NAME))) {
			throw 5UL;
		}

	} catch (const DWORD dwErrorStep) {
		// エラー発生
		TCHAR szDebugOut[1024];
		::wsprintf(szDebugOut, TEXT("%s: CBonTuner::OpenTuner() dwErrorStep = %lu\n"), TUNER_NAME, dwErrorStep);
		::OutputDebugString(szDebugOut);

		CloseTuner();
		return FALSE;
	}

	// チャンネル情報更新
	m_dwCurSpace = dwSpace;
	m_dwCurChannel = dwChannel;

	// TSデータパージ
	PurgeTsStream();

	return TRUE;
}

// 信号レベル(ビットレート)取得
const float CBonTuner::GetSignalLevel(void)
{
	CalcBitRate();
	return m_fBitRate;
}

void CBonTuner::CalcBitRate()
{
	DWORD dwCurrentTick = GetTickCount();
	DWORD Span = DiffTime(m_dwLastCalcTick,dwCurrentTick);

	if (Span >= BITRATE_CALC_TIME) {
		m_fBitRate = (float)(((double)m_dwRecvBytes*(8*1000))/((double)Span*(1024*1024)));
		m_dwRecvBytes = 0;
		m_dwLastCalcTick = dwCurrentTick;
	}
	return;
}

void CBonTuner::GetApiChannels(const char* space, picojson::value *channel_json)
{
	HttpClient client;
	char url[512];
	sprintf(url, "http://%s:%s/api/channels/%s", g_ServerHost, g_ServerPort, space);
	HttpResponse response = client.get(url);

	picojson::value v;
	std::string err = picojson::parse(v, response.content);
	if (err.empty()) {
		*channel_json = v;

		/*
		// DEBUG
		picojson::array channel_array = v.get<picojson::array>();
		for (picojson::array::iterator it = channel_array.begin(); it != channel_array.end(); it++) {
			picojson::object& channel = it->get<picojson::object>();
			std::string channel_name = channel["name"].get<std::string>();
			picojson::array& services = channel["services"].get<picojson::array>();

			std::string tmp_debug("channel name : ");
			tmp_debug.append(channel_name);
			wchar_t debug[128];
			mbstowcs(debug, tmp_debug.c_str(), sizeof(debug));
			OutputDebugString(debug);
		}
		*/
	}

}