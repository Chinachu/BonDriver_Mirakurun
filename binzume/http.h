#ifndef HTTP_H
#define HTTP_H
#include <string>
#include <vector>
#include <map>
#include "socket.h"

namespace Net
{
std::string urlencode(const std::string &str)
{
	std::string s;
	for (std::string::const_iterator it = str.begin();it!=str.end();++it) {
		char c = *it;
		char h[8];
		if (c>='A' && c<='Z' || c>='a' && c<='z' || c>='0' && c<='9') {
			s+=c;
		} else {
			sprintf(h,"%%%02x",c);
			s+=h;
		}
	}
	return s;
}
std::string urldecode(const std::string &str)
{
	std::string s;
	for (std::string::const_iterator it = str.begin();it!=str.end();++it) {
		char c = *it;
		char h[8];
		if (c!='%') {
			s+=c;
		} else {
			// メモリ内で連続じゃ無いかも知れないのでコピー
			h[0]=*++it;
			h[1]=*++it;
			h[2]=0;
			int d;
			sscanf(h,"%02x",&d);
			s+=(char)d;
		}
	}
	return s;
}

class HttpResponse
{
public:
	int status;
	std::map<std::string, std::vector<std::string> > headers;
	std::string content;
	std::string getHeader(const std::string &name) {
		if (headers.count(name)) {
			return headers.find(name)->second[0];
		}
		return "";
	}
	void clear() {
		status = 0;
		headers.clear();
		content = "";
	}
};

class HttpClient
{
public:
	enum METHOD {
		AUTO,
		GET,
		POST,
		HEAD,
	};
	METHOD method;

	// old
	int status;
	std::string body;
	std::map<std::string, std::vector<std::string> > headers;
	std::map<std::string,std::string> header;
	std::map<std::string,std::string> req_header;
	std::string cookie;

	HttpClient()
	{
		method = AUTO;
	}

	void clear()
	{
		body.clear();
		header.clear();
		headers.clear();
		method = AUTO;
		req_header.clear();
		cookie.clear();
	}

	Socket request(const std::string &host, int port,const std::string &path ,const std::string &data="")
	{
		status=0;
		header.clear();
		headers.clear();

		if (!req_header.count("Host")) req_header["Host"] = host;
		std::string methodstr;
		if (method==AUTO) {
			methodstr=data.size()?"POST":"GET";
		} else {
			methodstr=(method==POST)?"POST":"GET";
		}
		req_header["Connection"] = "close";

		Socket soc(host,port);
		if (method==GET && data.size()) {
			soc.write(methodstr+" "+path+"?"+data+" HTTP/1.1\r\n");
		} else {
			soc.write(methodstr+" "+path+" HTTP/1.1\r\n");
		}

		if (method==POST || method==AUTO&&data.size()) {
			char s[30];
			sprintf(s,"Content-Length: %d\r\n", data.size());
			soc.write(s);
			soc.write("Content-Type: application/x-www-form-urlencoded\r\n");
		}
		if (cookie != "") {
			soc.write(std::string("Cookie: ")+cookie+"\r\n");
		}

		for (std::map<std::string,std::string>::iterator it=req_header.begin();it!=req_header.end();++it) {
			soc.write(it->first+": "+it->second+"\r\n");
		}
		soc.write("\r\n" );
		if (soc.error()) return soc;

		if (method!=GET && data.size()) {
			soc.write(data);
		}

		std::string line;
		line = soc.readLine(); // HTTP status
		int p=line.find(" ");
		if (p!=std::string::npos) {
			status = atoi(line.c_str()+p+1);
		}
		while(!soc.error()) {
			line = soc.readLine();
#ifdef CPPFL_DEBUG
			std::cout << line << std::endl;
#endif
			if (line.size()==0) break;
			if (line.size() && line[line.size()-1]=='\r') line.resize(line.size()-1);
			if (line.empty()) break;
			int p=line.find(":");
			std::string name=line.substr(0, p);
			if (line[p+1]==' ') p++;
			std::string value=line.substr(p+1);
			header[name]=value;
			headers[name].push_back(value);
			//cout << name << " : " << value << endl;
		}

		if (headers.count("Set-Cookie")) {
			std::string c = headers["Set-Cookie"][0];
			size_t p = c.find(";");
			if (p != std::string::npos) {
				c.resize(p);
			}
			cookie.swap(c);
		}

		return soc;
	}
	
	Socket request(const std::string &url, const std::string &data="")
	{
		using namespace std;
		int s=0;
		if (url.substr(0,7)=="http://") s=7;
		int p=url.find("/",s);
		string host = url.substr(s, p-s);
		string path = url.substr(p);
		int port=80;

		p=host.find(":");
		if (p!=string::npos) {
			port=atoi(host.substr(p+1).c_str());
			host = host.substr(0, p);
		}
		return request(host, port, path, data);
	}

	// old method
	int load(const std::string &url, const std::string &data="")
	{
		Socket soc = request(url, data);
		while(!soc.error()) {
			body += soc.read();
		}
		soc.close();
		return body.size();
	}

	// old method
	int load(const std::string &url, const std::map<std::string,std::string> &params)
	{
		std::string data;
		for (std::map<std::string,std::string>::const_iterator it = params.begin();it!=params.end();++it) {
			if (data!="") data+="&";
			data += (*it).first;
			data += '=';
			data+= urlencode((*it).second);
		}
		return load(url,data);
	}

	int load(HttpResponse &res, const std::string &url, const std::string &postdata="")
	{
		Socket soc = request(url, postdata);
		res.status = status;
		res.content.clear();
		res.headers.swap(headers);
		while(!soc.error()) {
			res.content.append(soc.read());
		}
		soc.close();
		return res.status == 200;
	}

	int get(HttpResponse &res, const std::string &url, const std::map<std::string,std::string> &params = std::map<std::string,std::string>())
	{
		std::string data = url;
		bool is_first = true;
		for (std::map<std::string,std::string>::const_iterator it = params.begin();it!=params.end();++it) {
			data+= is_first?"?":"&";
			is_first = false;
			data += (*it).first;
			data += '=';
			data += urlencode((*it).second);
		}
		return load(res, data);
	}

	int post(HttpResponse &res, const std::string &url, const std::map<std::string,std::string> &params)
	{
		std::string data;
		for (std::map<std::string,std::string>::const_iterator it = params.begin();it!=params.end();++it) {
			if (data!="") data+="&";
			data += (*it).first;
			data += '=';
			data+= urlencode((*it).second);
		}
		return load(res, url, data);
	}

	HttpResponse get(const std::string &url, const std::map<std::string,std::string> &params = std::map<std::string,std::string>())
	{
		HttpResponse res;
		get(res, url, params);
		return res;
	}

	HttpResponse post(const std::string &url, const std::map<std::string,std::string> &params = std::map<std::string,std::string>())
	{
		HttpResponse res;
		post(res, url, params);
		return res;
	}

	std::string get_content(const std::string &url) {
		HttpResponse res;
		get(res, url);
		return res.content;
	}


};

}
#endif
