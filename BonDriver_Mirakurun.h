#include <winsock2.h>
#include <ws2tcpip.h>
#include <InitGuid.h>
#include "IBonDriver2.h"
#include "binzume\socket.h"
#include "binzume\http.h"
#include "picojson\picojson.h"
using namespace std;
using namespace Net;

#if !defined(_BONTUNER_H_)
#define _BONTUNER_H_

#if _MSC_VER > 1000
#pragma once
#endif // _MSC_VER > 1000

#define dllimport dllexport


#define TUNER_NAME "BonDriver_Mirakurun"

// 受信サイズ
#define TSDATASIZE	48128	// TSデータのサイズ 188 * 256

static wchar_t g_IniFilePath[MAX_PATH] = { '\0' };

#define MAX_HOST_LEN	256
#define MAX_PORT_LEN	8
static char g_ServerHost[MAX_HOST_LEN];
static char g_ServerPort[MAX_PORT_LEN];
static int g_DecodeB25;
static int g_Priority;
picojson::value g_Channel_JSON_GR;
picojson::value g_Channel_JSON_BS;
picojson::value g_Channel_JSON_CS;
static int g_MagicPacket_Enable;
static char g_MagicPacket_TargetMAC[18];
static char g_MagicPacket_TargetIP[16];

class CBonTuner : public IBonDriver2
{
public:
	CBonTuner();
	virtual ~CBonTuner();

// IBonDriver
	const BOOL OpenTuner(void);
	void CloseTuner(void);

	const BOOL SetChannel(const BYTE bCh);
	const float GetSignalLevel(void);

	const DWORD WaitTsStream(const DWORD dwTimeOut = 0);
	const DWORD GetReadyCount(void);

	const BOOL GetTsStream(BYTE *pDst, DWORD *pdwSize, DWORD *pdwRemain);
	const BOOL GetTsStream(BYTE **ppDst, DWORD *pdwSize, DWORD *pdwRemain);

	void PurgeTsStream(void);

// IBonDriver2(暫定)
	LPCTSTR GetTunerName(void);

	const BOOL IsTunerOpening(void);

	LPCTSTR EnumTuningSpace(const DWORD dwSpace);
	LPCTSTR EnumChannelName(const DWORD dwSpace, const DWORD dwChannel);

	const BOOL SetChannel(const DWORD dwSpace, const DWORD dwChannel);

	const DWORD GetCurSpace(void);
	const DWORD GetCurChannel(void);

	void Release(void);

	static CBonTuner * m_pThis;
	static HINSTANCE m_hModule;
	static char * m_cList[7];


protected:
	// I/Oリクエストキューデータ
	struct AsyncIoReq
	{
		WSAOVERLAPPED OverLapped;
		DWORD dwState;
		DWORD dwRxdSize;
		BYTE RxdBuff[TSDATASIZE];
		AsyncIoReq *pNext;
	};

	AsyncIoReq * AllocIoReqBuff(const DWORD dwBuffNum);
	void FreeIoReqBuff(AsyncIoReq *pBuff);

	static DWORD WINAPI PushIoThread(LPVOID pParam);
	static DWORD WINAPI PopIoThread(LPVOID pParam);

	const BOOL PushIoRequest(SOCKET sock);
	const BOOL PopIoRequest(SOCKET sock);

	bool m_bTunerOpen;

	HANDLE m_hMutex;

	BYTE m_RxdBuff[256];

	AsyncIoReq *m_pIoReqBuff;
	AsyncIoReq *m_pIoPushReq;
	AsyncIoReq *m_pIoPopReq;
	AsyncIoReq *m_pIoGetReq;

	DWORD m_dwBusyReqNum;
	DWORD m_dwReadyReqNum;

	HANDLE m_hPushIoThread;
	HANDLE m_hPopIoThread;
	BOOL m_bLoopIoThread;

	HANDLE m_hOnStreamEvent;

	CRITICAL_SECTION m_CriticalSection;

	DWORD m_dwCurSpace;
	DWORD m_dwCurChannel;

	// 追加 byMeru(2008/03/27)
	SOCKET m_sock;
	float m_fBitRate;

	void CalcBitRate();
	void GetApiChannels(const char* space, picojson::value *json_array);
	DWORD m_dwRecvBytes;
	DWORD m_dwLastCalcTick;
	ULONGLONG m_u64RecvBytes;
	ULONGLONG m_u64LastCalcByte;


};

#endif // !defined(_BONTUNER_H_)
