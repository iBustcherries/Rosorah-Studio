"""Turns down passkey requests in Rosorah's browser windows. Twitch's sign-in page asks for a passkey
as soon as it opens, and on Windows the browser hands that to Windows Hello, which pops up a
Windows Security dialog over the stream. With passkeys unavailable, sites sign in with a password."""
JS=r'''(function(){
var C=window.CredentialsContainer&&CredentialsContainer.prototype;
if(C)['get','create'].forEach(function(m){var o=C[m];if(typeof o!=='function')return;
 Object.defineProperty(C,m,{configurable:true,writable:true,value:function(opts){
  if(opts&&opts.publicKey)return Promise.reject(new DOMException('Passkeys are not available here','NotAllowedError'));
  return o.apply(this,arguments);}});});
var P=window.PublicKeyCredential;
if(P){P.isConditionalMediationAvailable=function(){return Promise.resolve(false)};
 P.isUserVerifyingPlatformAuthenticatorAvailable=function(){return Promise.resolve(false)};}
})();'''
def install(profile):
    """For every page and frame of this profile, before the page's own scripts run."""
    from PySide6.QtWebEngineCore import QWebEngineScript
    script=QWebEngineScript();script.setName('rosorah-no-passkeys');script.setSourceCode(JS)
    script.setInjectionPoint(QWebEngineScript.InjectionPoint.DocumentCreation)
    script.setWorldId(QWebEngineScript.ScriptWorldId.MainWorld);script.setRunsOnSubFrames(True)
    profile.scripts().insert(script)
