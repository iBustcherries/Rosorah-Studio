#!/usr/bin/env python3
"""Isolated canvas browser. Binary packets to parent; no listening server."""
import os, sys, json, struct, threading, subprocess, signal
from collections import deque
from urllib.parse import urlsplit

class Output:
    def __init__(self):
        self.cv=threading.Condition();self.frame=None;self.audio=deque(maxlen=10);self.status=deque(maxlen=8);self.closed=False
        threading.Thread(target=self.write,daemon=True).start()
    def put(self,kind,payload,w=0,h=0):
        with self.cv:
            packet=(kind,w,h,payload)
            if kind==1:self.frame=packet
            elif kind==2:self.audio.append(packet)
            else:self.status.append(packet)
            self.cv.notify()
    def write(self):
        try:
            while True:
                with self.cv:
                    self.cv.wait_for(lambda:self.closed or self.frame is not None or self.audio or self.status)
                    if self.closed:return
                    packets=list(self.status)+list(self.audio);self.status.clear();self.audio.clear()
                    if self.frame is not None:packets.append(self.frame);self.frame=None
                for k,w,h,data in packets:
                    sys.stdout.buffer.write(struct.pack('<4sIIII',b'RSBW',k,w,h,len(data)));sys.stdout.buffer.write(data)
                sys.stdout.buffer.flush()
        except (OSError,BrokenPipeError):self.closed=True
    def note(self,text):self.put(3,text.encode('utf-8')[:4096])

class Audio:
    def __init__(self,out,enabled):
        self.module=None;self.process=None;self.out=out;self.ready=False
        if not enabled:return
        self.sink='rosorah_browser_'+str(os.getpid())
        try:
            load=subprocess.run(['pactl','load-module','module-null-sink','sink_name='+self.sink,'rate=48000','channels=2','sink_properties=device.description=Rosorah_Browser'],capture_output=True,timeout=4)
            if load.returncode:raise RuntimeError()
            self.module=int(load.stdout.strip())
            os.environ['PULSE_SINK']=self.sink
            self.process=subprocess.Popen(['parec','--device='+self.sink+'.monitor','--format=float32le','--rate=48000','--channels=2','--latency-msec=20'],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
            self.ready=True;threading.Thread(target=self.read,daemon=True).start()
        except (OSError,ValueError,RuntimeError,subprocess.TimeoutExpired):
            self.close();out.note('Browser audio unavailable: install pulseaudio-utils and check PipeWire Pulse. Page muted.')
    def read(self):
        p=self.process
        while p:
            data=p.stdout.read(3840)
            if not data:break
            if len(data)==3840:self.out.put(2,data)
        self.ready=False
        self.out.note('Browser audio capture stopped; reload the source to reconnect.')
    def close(self):
        if self.process:
            self.process.terminate()
            try:self.process.wait(timeout=1)
            except subprocess.TimeoutExpired:self.process.kill();self.process.wait()
            self.process=None
        if self.module is not None:
            try:subprocess.run(['pactl','unload-module',str(self.module)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=2)
            except (OSError,subprocess.TimeoutExpired):pass
            self.module=None

# window.obsstudio as OBS provides it at its "read access" level. STATE is the starting state.
OBS_API=r'''(function(){
var s=STATE;
function fire(n,d){try{window.dispatchEvent(new CustomEvent(n,{detail:d}))}catch(e){}}
function scene(){return {name:s.scene,width:s.width,height:s.height}}
window.obsstudio={pluginVersion:'2.24.0',
getControlLevel:function(cb){cb(1)},
getStatus:function(cb){cb({recording:s.recording,recordingPaused:false,streaming:s.streaming,replaybuffer:s.replaybuffer,virtualcam:s.virtualcam})},
getCurrentScene:function(cb){cb(scene())},
getScenes:function(cb){cb([s.scene])},
getTransitions:function(cb){cb([])},
getCurrentTransition:function(cb){cb('')}};
var started={streaming:'obsStreamingStarted',recording:'obsRecordingStarted',replaybuffer:'obsReplaybufferStarted',virtualcam:'obsVirtualcamStarted'};
var stopped={streaming:'obsStreamingStopped',recording:'obsRecordingStopped',replaybuffer:'obsReplaybufferStopped',virtualcam:'obsVirtualcamStopped'};
window.__rosorahState=function(c){
for(var k in c)s[k]=c[k];
if('visible' in c){
fire('obsSourceVisibleChanged',{visible:s.visible});fire('obsSourceActiveChanged',{active:s.visible});
var o=window.obsstudio;if(typeof o.onVisibilityChange==='function')o.onVisibilityChange(s.visible);if(typeof o.onActiveChange==='function')o.onActiveChange(s.visible);}
if('scene' in c)fire('obsSceneChanged',scene());
for(var k in started)if(k in c)fire(c[k]?started[k]:stopped[k]);};
})();'''
CSS_SCRIPT=r'''(function(){var st=document.createElement('style');st.id='rosorah-custom-css';st.textContent=CSS;(document.head||document.documentElement).appendChild(st);})();'''

def main():
    config=json.loads(sys.stdin.readline())
    scheme=urlsplit(config['url']).scheme
    if scheme not in ('https','http','file'):raise ValueError('Use an HTTP or HTTPS overlay URL or a local file')
    w,h,fps=int(config['width']),int(config['height']),int(config['fps'])
    if not (16<=w<=1920 and 16<=h<=1920 and w*h<=1920*1080 and 1<=fps<=60):raise ValueError('Invalid browser dimensions or frame rate')
    # Qt is initialized only after the dedicated Pulse sink is selected.
    # On Windows the page plays normally and Rosorah records this process's sound itself.
    system_audio=bool(config.get('system_audio')) and bool(config.get('audio'))
    out=Output();sound=Audio(out,config.get('audio',False) and not system_audio)
    import atexit
    atexit.register(sound.close)
    from PySide6.QtCore import Qt,QTimer,QUrl,QObject,Signal
    from PySide6.QtGui import QColor,QImage
    from PySide6.QtWidgets import QApplication
    from PySide6.QtWebEngineCore import QWebEnginePage,QWebEngineProfile,QWebEngineScript,QWebEngineSettings
    from PySide6.QtWebEngineWidgets import QWebEngineView
    class Inbox(QObject):line=Signal(str)
    class Page(QWebEnginePage):
        def javaScriptConsoleMessage(self,*args):pass
        def createWindow(self,*args):return None
        def javaScriptAlert(self,*args):pass
        def javaScriptConfirm(self,*args):return False
        def javaScriptPrompt(self,*args):return (False,'')
    class View(QWebEngineView):
        def closeEvent(self,event):
            event.ignore();self.hide();self.setAttribute(Qt.WidgetAttribute.WA_DontShowOnScreen,True);self.show();self.page().setVisible(True)
    app=QApplication([]);app.setQuitOnLastWindowClosed(False)
    view=View();view.setWindowTitle('Rosorah · Interact with browser source');view.setFixedSize(w,h)
    view.setAttribute(Qt.WidgetAttribute.WA_DontShowOnScreen,True)
    view.setAttribute(Qt.WidgetAttribute.WA_TranslucentBackground,True)
    # Each source keeps its own sign-ins, apart from Rosorah's Twitch/YouTube account cookies.
    storage=config.get('storage') or ''
    if storage:
        os.makedirs(storage,exist_ok=True)
        profile=QWebEngineProfile('rosorah-browser-source',view);profile.setPersistentStoragePath(storage);profile.setCachePath(os.path.join(storage,'cache'))
        profile.setPersistentCookiesPolicy(QWebEngineProfile.PersistentCookiesPolicy.ForcePersistentCookies)
    else:profile=QWebEngineProfile(view)
    profile.setHttpCacheMaximumSize(32*1024*1024)
    profile.downloadRequested.connect(lambda item:item.cancel())
    page=Page(profile,view);view.setPage(page);page.setBackgroundColor(QColor(0,0,0,0));page.setAudioMuted(not (sound.ready or system_audio))
    # Overlays made for OBS: its stylesheet box and its window.obsstudio API.
    state={'visible':False,'streaming':False,'recording':False,'replaybuffer':False,'virtualcam':False,'scene':'','width':1920,'height':1080}
    def api():
        return OBS_API.replace('STATE',json.dumps(state))
    def add_script(name,source,point):
        script=QWebEngineScript();script.setName(name);script.setSourceCode(source)
        script.setInjectionPoint(point);script.setWorldId(QWebEngineScript.ScriptWorldId.MainWorld);script.setRunsOnSubFrames(False)
        old=page.scripts().find(name)
        for o in old:page.scripts().remove(o)
        page.scripts().insert(script)
    add_script('rosorah-obs',api(),QWebEngineScript.InjectionPoint.DocumentCreation)
    css=str(config.get('css') or '')
    if css.strip():add_script('rosorah-css',CSS_SCRIPT.replace('CSS',json.dumps(css)),QWebEngineScript.InjectionPoint.DocumentReady)
    page.permissionRequested.connect(lambda permission:permission.deny())
    page.settings().setAttribute(QWebEngineSettings.WebAttribute.PlaybackRequiresUserGesture,False)
    # A local overlay may load its own pictures and sounds and reach its web services, as in OBS.
    local=scheme=='file'
    page.settings().setAttribute(QWebEngineSettings.WebAttribute.LocalContentCanAccessFileUrls,local)
    page.settings().setAttribute(QWebEngineSettings.WebAttribute.LocalContentCanAccessRemoteUrls,local)
    page.renderProcessTerminated.connect(lambda *args:(out.note('Browser renderer stopped. Reload this source.'),app.quit()))
    page.loadFinished.connect(lambda ok:out.note(('Browser ready'+(' · isolated audio' if sound.ready else ' · sound recorded by Rosorah' if system_audio else ' · muted')) if ok else 'Page could not load. Check its address or refresh.'))
    view.show();page.setVisible(True);page.setLifecycleState(QWebEnginePage.LifecycleState.Active);view.load(QUrl(config['url']))
    def update(changes):
        changes={k:v for k,v in changes.items() if k in state and v!=state[k]}
        if not changes:return
        state.update(changes)
        add_script('rosorah-obs',api(),QWebEngineScript.InjectionPoint.DocumentCreation)
        page.runJavaScript('window.__rosorahState&&window.__rosorahState(%s)'%json.dumps(changes))
    last=[None]
    def capture():
        if config.get('audio') and not sound.ready:page.setAudioMuted(True)
        img=view.grab().toImage().convertToFormat(QImage.Format.Format_RGBA8888)
        if img.width()!=w or img.height()!=h:img=img.scaled(w,h)
        data=bytes(img.constBits())
        if data!=last[0]:out.put(1,data,w,h);last[0]=data
    timer=QTimer();timer.timeout.connect(capture);timer.start(max(1,round(1000/fps)))
    inbox=Inbox()
    def command(line):
        try:message=json.loads(line);kind=message.get('type')
        except (ValueError,AttributeError):return
        if kind=='quit':app.quit()
        elif kind=='reload':view.reload()
        elif kind=='visible':update({'visible':bool(message.get('visible'))})
        elif kind=='state' and isinstance(message.get('state'),dict):update(message['state'])
        elif kind=='forget':
            profile.cookieStore().deleteAllCookies();profile.clearHttpCache();profile.clearAllVisitedLinks();view.reload()
            out.note('Sign-ins for this source were cleared')
        elif kind=='interact':view.hide();view.setAttribute(Qt.WidgetAttribute.WA_DontShowOnScreen,False);view.showNormal();view.activateWindow()
    inbox.line.connect(command)
    def read():
        for line in sys.stdin:
            if len(line)<4096:inbox.line.emit(line)
        inbox.line.emit('{"type":"quit"}')
    threading.Thread(target=read,daemon=True).start()
    signal.signal(signal.SIGTERM,lambda *args:app.quit())
    try:return app.exec()
    finally:
        timer.stop();page.setAudioMuted(True);sound.close();view.hide()
        import shiboken6
        shiboken6.delete(page);shiboken6.delete(view)
        with out.cv:out.closed=True;out.cv.notify_all()
if __name__=='__main__':sys.exit(main())
