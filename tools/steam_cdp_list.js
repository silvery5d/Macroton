// Run with Wine's cscript.exe inside the same prefix as Windows Steam.
// Steam's CEF debug endpoint is intentionally bound to the Wine network
// namespace, so a host-side HTTP client may not be able to reach it.
var request = new ActiveXObject("MSXML2.XMLHTTP.6.0");
request.open("GET", "http://127.0.0.1:8080/json/list", false);
request.send();
if (request.status !== 200) {
    WScript.StdErr.WriteLine("Steam CEF returned HTTP " + request.status);
    WScript.Quit(1);
}
WScript.Echo(request.responseText);
