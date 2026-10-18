#!/usr/bin/env python3
"""Isolated canvas browser. Binary packets to parent; no listening server."""
import os, sys, json, struct, threading, subprocess, signal, time
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

def main():
    config=json.loads(sys.stdin.readline())
    if urlsplit(config['url']).scheme not in ('https','http'):raise ValueError('Use an HTTP or HTTPS overlay URL')
    w,h,fps=int(config['width']),int(config['height']),int(config['fps'])
    if not (16<=w<=1920 and 16<=h<=1920 and w*h<=1920*1080 and 1<=fps<=60):raise ValueError('Invalid browser dimensions or frame rate')
    # Qt is initialized only after the dedicated Pulse sink is selected.
    out=Output();sound=Audio(out,config.get('audio',False))
    import atexit
    atexit.register(sound.close)
    from PySide6.QtCore import Qt,QTimer,QUrl,QObject,Signal
    from PySide6.QtGui import QColor,QImage
    from PySide6.QtWidgets import QApplication
    from PySide6.QtWebEngineCore import QWebEnginePage,QWebEngineProfile,QWebEngineSettings
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
    # Off-the-record: overlay pages never inherit Twitch/YouTube account cookies.
    profile=QWebEngineProfile(view);profile.setHttpCacheMaximumSize(32*1024*1024)
    profile.downloadRequested.connect(lambda item:item.cancel())
    page=Page(profile,view);view.setPage(page);page.setBackgroundColor(QColor(0,0,0,0));page.setAudioMuted(not sound.ready)
    page.permissionRequested.connect(lambda permission:permission.deny())
    page.settings().setAttribute(QWebEngineSettings.WebAttribute.PlaybackRequiresUserGesture,False)
    page.settings().setAttribute(QWebEngineSettings.WebAttribute.LocalContentCanAccessFileUrls,False)
    page.settings().setAttribute(QWebEngineSettings.WebAttribute.LocalContentCanAccessRemoteUrls,False)
    page.renderProcessTerminated.connect(lambda *args:(out.note('Browser renderer stopped. Reload this source.'),app.quit()))
    page.loadFinished.connect(lambda ok:out.note(('Browser ready'+(' · isolated audio' if sound.ready else ' · muted')) if ok else 'Page could not load. Check its URL or refresh.'))
    view.show();page.setVisible(True);page.setLifecycleState(QWebEnginePage.LifecycleState.Active);view.load(QUrl(config['url']))
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
        try:kind=json.loads(line).get('type')
        except (ValueError,AttributeError):return
        if kind=='quit':app.quit()
        elif kind=='reload':view.reload()
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
