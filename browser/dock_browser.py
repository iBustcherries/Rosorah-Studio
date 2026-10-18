#!/usr/bin/env python3
"""A web page drawn inside a studio panel. The page renders off screen; frames go to stdout as
packets and the panel's mouse and keyboard input arrives as JSON lines on stdin."""
import json, os, signal, struct, sys, threading
from urllib.parse import urlsplit

MAGIC=b'RSDK'
def put(kind,payload=b'',w=0,h=0):
    # kind 1: RGBA frame (w×h), 2: page title, 3: status note, 4: current address
    sys.stdout.buffer.write(MAGIC+struct.pack('<IIII',kind,w,h,len(payload))+payload);sys.stdout.buffer.flush()

def main():
    config=json.loads(sys.stdin.readline())
    if urlsplit(config['url']).scheme not in ('https','http'):raise ValueError('Use an HTTP or HTTPS address')
    from PySide6.QtCore import Qt,QTimer,QUrl,QObject,Signal,QPointF,QPoint,QEvent,QSize
    from PySide6.QtGui import QImage,QMouseEvent,QWheelEvent,QKeyEvent,QFocusEvent
    from PySide6.QtWidgets import QApplication
    from PySide6.QtWebEngineCore import QWebEnginePage,QWebEngineProfile,QWebEngineSettings
    from PySide6.QtWebEngineWidgets import QWebEngineView
    class Inbox(QObject):line=Signal(str)
    class Page(QWebEnginePage):
        def javaScriptConsoleMessage(self,*args):pass
        # Links that open a new window load in the panel instead.
        def createWindow(self,*args):return self
    app=QApplication([]);app.setQuitOnLastWindowClosed(False)
    view=QWebEngineView()
    view.setAttribute(Qt.WidgetAttribute.WA_DontShowOnScreen,True)
    # One saved profile for every web panel, apart from the chat window's accounts, so sign-ins stick.
    profile=QWebEngineProfile('rosorah-docks',view)
    profile.setPersistentStoragePath(config['storage']);profile.setCachePath(os.path.join(config['storage'],'cache'))
    profile.setHttpCacheMaximumSize(64*1024*1024)
    profile.setPersistentCookiesPolicy(QWebEngineProfile.PersistentCookiesPolicy.ForcePersistentCookies)
    profile.downloadRequested.connect(lambda item:item.cancel())
    page=Page(profile,view);view.setPage(page)
    page.permissionRequested.connect(lambda permission:permission.deny())
    page.settings().setAttribute(QWebEngineSettings.WebAttribute.LocalContentCanAccessFileUrls,False)
    page.renderProcessTerminated.connect(lambda *args:put(3,b'The page stopped. Reload it.'))
    page.loadFinished.connect(lambda ok:put(3,b'' if ok else b'The page could not load. Check its address.'))
    page.titleChanged.connect(lambda t:put(2,t.encode('utf-8')[:512]))
    page.urlChanged.connect(lambda u:put(4,u.toString().encode('utf-8')[:2048]))
    size=[max(16,min(3840,int(config['width']))),max(16,min(2160,int(config['height'])))]
    view.resize(*size);page.setZoomFactor(max(0.25,min(5.0,float(config.get('zoom',1.0)))))
    view.show();page.setVisible(True);page.setLifecycleState(QWebEnginePage.LifecycleState.Active);view.load(QUrl(config['url']))
    last=[None]
    def capture():
        img=view.grab().toImage().convertToFormat(QImage.Format.Format_RGBA8888)
        if img.width()!=size[0] or img.height()!=size[1]:img=img.scaled(size[0],size[1])
        data=bytes(img.constBits())
        if data!=last[0]:put(1,data,size[0],size[1]);last[0]=data
    timer=QTimer();timer.timeout.connect(capture);timer.start(33)
    def target():return view.focusProxy() or view
    buttons={'left':Qt.MouseButton.LeftButton,'right':Qt.MouseButton.RightButton,'middle':Qt.MouseButton.MiddleButton,
             'back':Qt.MouseButton.BackButton,'forward':Qt.MouseButton.ForwardButton}
    held=[Qt.MouseButton.NoButton]
    def mods(m):
        out=Qt.KeyboardModifier.NoModifier
        if m.get('shift'):out|=Qt.KeyboardModifier.ShiftModifier
        if m.get('ctrl'):out|=Qt.KeyboardModifier.ControlModifier
        if m.get('alt'):out|=Qt.KeyboardModifier.AltModifier
        return out
    def focus():
        view.setFocus();w=target();w.setFocus()
        app.sendEvent(w,QFocusEvent(QEvent.Type.FocusIn,Qt.FocusReason.MouseFocusReason))
    keys={k:getattr(Qt.Key,'Key_'+k) for k in ('Enter','Escape','Tab','Backspace','Delete','Insert','Home','End','PageUp','PageDown','Left','Right','Up','Down','Space')}
    keys['Enter']=Qt.Key.Key_Return
    def command(line):
        try:c=json.loads(line);kind=c.get('type')
        except (ValueError,AttributeError):return
        if kind=='quit':app.quit()
        elif kind=='reload':view.reload()
        elif kind=='pause':timer.stop()
        elif kind=='resume':last[0]=None;timer.start(33)
        elif kind=='back':view.back()
        elif kind=='forward':view.forward()
        elif kind=='resize':
            size[0]=max(16,min(3840,int(c['width'])));size[1]=max(16,min(2160,int(c['height'])))
            view.resize(*size);page.setZoomFactor(max(0.25,min(5.0,float(c.get('zoom',1.0)))))
        elif kind in ('move','press','release'):
            pos=QPointF(float(c['x']),float(c['y']));b=buttons.get(c.get('button'),Qt.MouseButton.NoButton)
            if kind=='press':focus();held[0]|=b;t=QEvent.Type.MouseButtonPress
            elif kind=='release':held[0]&=~b;t=QEvent.Type.MouseButtonRelease
            else:t=QEvent.Type.MouseMove;b=Qt.MouseButton.NoButton
            app.sendEvent(target(),QMouseEvent(t,pos,view.mapToGlobal(pos),b,held[0],mods(c)))
        elif kind=='wheel':
            pos=QPointF(float(c['x']),float(c['y']));dx,dy=int(c.get('dx',0)),int(c.get('dy',0))
            app.sendEvent(target(),QWheelEvent(pos,view.mapToGlobal(pos),QPoint(dx,dy),QPoint(dx*2,dy*2),held[0],mods(c),Qt.ScrollPhase.NoScrollPhase,False))
        elif kind=='key':
            key=keys.get(c.get('key',''))
            if key is None and len(c.get('key',''))==1:key=getattr(Qt.Key,'Key_'+c['key'].upper(),None)
            if key is None:return
            text={'Enter':'\r','Tab':'\t','Backspace':'\b','Escape':'\x1b','Space':' '}.get(c['key'],'')
            if c.get('ctrl') or c.get('alt'):text=''
            t=QEvent.Type.KeyPress if c.get('down',True) else QEvent.Type.KeyRelease
            app.sendEvent(target(),QKeyEvent(t,key,mods(c),text))
        elif kind=='text':
            # Typed characters (and paste) go in as text the way a keyboard would deliver them.
            text=str(c.get('text',''))[:10000]
            if len(text)==1:
                for t in (QEvent.Type.KeyPress,QEvent.Type.KeyRelease):app.sendEvent(target(),QKeyEvent(t,0,Qt.KeyboardModifier.NoModifier,text))
            elif text:page.runJavaScript('document.execCommand("insertText",false,'+json.dumps(text)+')')
    inbox=Inbox();inbox.line.connect(command)
    def read():
        for line in sys.stdin:
            if len(line)<20000:inbox.line.emit(line)
        inbox.line.emit('{"type":"quit"}')
    threading.Thread(target=read,daemon=True).start()
    signal.signal(signal.SIGTERM,lambda *args:app.quit())
    try:return app.exec()
    finally:
        timer.stop();view.hide()
        import shiboken6
        shiboken6.delete(page);shiboken6.delete(view)
if __name__=='__main__':sys.exit(main())
