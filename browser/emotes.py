"""Bounded, async platform image cache. Browser keeps original animation; native dock gets decoded frames."""
import base64, threading
from collections import OrderedDict
from urllib.parse import urlsplit
from PySide6.QtCore import QObject, Signal, QByteArray, QBuffer, QIODevice, QTimer, Qt
from PySide6.QtGui import QImageReader
from PySide6.QtNetwork import QNetworkAccessManager, QNetworkRequest
from PySide6.QtCore import QUrl
HOSTS={'static-cdn.jtvnw.net','yt3.ggpht.com','yt3.googleusercontent.com','www.gstatic.com','fonts.gstatic.com','i.ytimg.com','files.kick.com'}
def safe_url(src):
    try:
        u=urlsplit(src)
        return len(src)<=2048 and u.scheme=='https' and (u.hostname in HOSTS or (u.hostname=='www.youtube.com' and u.path.startswith('/s/gaming/emoji/'))) and not u.username and not u.password and u.port in (None,443)
    except (ValueError,TypeError):return False
class Images(QObject):
    decoded=Signal(object)
    def __init__(self,parent,emit):
        super().__init__(parent);self.network=QNetworkAccessManager(self);self.seen=OrderedDict();self.active=0;self.waiting=[];self.emit=emit
        self.decoded.connect(self.complete)
    def request(self,src):
        if not safe_url(src) or src in self.seen:return
        self.seen[src]=True
        while len(self.seen)>256:self.seen.popitem(last=False)
        if len(self.waiting)<128:self.waiting.append(src)
        self.pump()
    def pump(self):
        while self.waiting and self.active<3:
            src=self.waiting.pop(0);self.active+=1
            request=QNetworkRequest(QUrl(src));request.setTransferTimeout(10000)
            request.setAttribute(QNetworkRequest.Attribute.RedirectPolicyAttribute,QNetworkRequest.RedirectPolicy.ManualRedirectPolicy)
            reply=self.network.get(request)
            reply.downloadProgress.connect(lambda received,total,r=reply:r.abort() if max(received,total)>2*1024*1024 else None)
            reply.finished.connect(lambda r=reply,s=src:self.downloaded(r,s))
    def downloaded(self,reply,src):
        data=bytes(reply.readAll());ok=reply.error()==reply.NetworkError.NoError and reply.attribute(QNetworkRequest.Attribute.HttpStatusCodeAttribute)==200
        reply.deleteLater()
        if ok and len(data)<=2*1024*1024:
            threading.Thread(target=self.decode,args=(src,data),daemon=True).start()
        else:self.complete([])
    def decode(self,src,data):
        frames=[]
        try:
            buf=QBuffer();buf.setData(QByteArray(data));buf.open(QIODevice.OpenModeFlag.ReadOnly)
            reader=QImageReader(buf);size=reader.size()
            if size.width()>2048 or size.height()>2048 or size.width()<=0:raise ValueError('size')
            total=0
            for index in range(120):
                frame=reader.read()
                if frame.isNull():break
                frame=frame.scaled(64,64,Qt.AspectRatioMode.KeepAspectRatio,Qt.TransformationMode.SmoothTransformation)
                output=QBuffer();output.open(QIODevice.OpenModeFlag.WriteOnly);frame.save(output,'PNG');png=bytes(output.data());total+=len(png)
                if total>2*1024*1024:break
                frames.append({'type':'emote','url':src,'index':index,'delay':max(20,min(10000,reader.nextImageDelay())),'png':base64.b64encode(png).decode('ascii')})
                if not reader.supportsAnimation() or not reader.canRead():break
        except Exception:pass
        self.decoded.emit(frames)
    def complete(self,frames):
        for frame in frames:self.emit(frame)
        self.active=max(0,self.active-1);self.pump()
