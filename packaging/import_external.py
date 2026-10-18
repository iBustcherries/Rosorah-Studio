"""Read-only, bounded OBS/Firebot import. Never extracts archives or runs scripts.

Input (stdin JSON): path (a file, ZIP or folder), media_folder, project (an empty Rosorah project).
Output: the converted project plus warnings for everything that could not be carried over."""
import json, sys, zipfile, configparser, math, struct
from pathlib import Path, PurePosixPath
LIMIT=20*1024*1024
TOTAL=64*1024*1024
SKIP=['auth','token','secret','account','service','stream_key','credentials']

def wanted(name):
    lower=name.lower()
    return lower.endswith(('.json','.ini')) and not any(s in PurePosixPath(lower).name for s in SKIP) and not lower.endswith('.bak')

def documents(path):
    path=Path(path)
    if path.is_dir():
        # An OBS or Firebot settings folder: read only its .json/.ini files, never anything else.
        result=[];total=0
        for i,p in enumerate(sorted(path.rglob('*'))):
            if i>20000:raise ValueError('Folder has more than 20,000 entries; choose the OBS or Firebot folder itself')
            rel=p.relative_to(path).as_posix()
            if not p.is_file() or p.is_symlink() or not wanted(rel) or any(x in rel.lower() for x in ['logs/','crashes/','updates/','plugin_config/','node_modules/']):continue
            size=p.stat().st_size
            if size>LIMIT:continue
            total+=size
            if total>TOTAL:raise ValueError('Folder exceeds import limits')
            result.append((rel,p.read_text(encoding='utf-8-sig',errors='replace')))
        return result
    if path.stat().st_size>128*1024*1024: raise ValueError('Backup exceeds 128 MiB')
    if zipfile.is_zipfile(path):
        with zipfile.ZipFile(path) as z:
            entries=z.infolist()
            if len(entries)>5000 or sum(e.file_size for e in entries)>256*1024*1024:raise ValueError('Backup exceeds import limits')
            result=[];total=0
            for e in entries:
                n=PurePosixPath(e.filename.replace('\\','/'))
                if n.is_absolute() or '..' in n.parts:raise ValueError('Unsafe archive path')
                if e.is_dir() or e.file_size>LIMIT or not wanted(str(n)):continue
                total+=e.file_size
                if total>TOTAL:raise ValueError('Backup exceeds import limits')
                result.append((str(n),z.read(e).decode('utf-8-sig',errors='replace')))
            return result
    if path.stat().st_size>LIMIT:raise ValueError('Configuration exceeds 20 MiB')
    return [(path.name,path.read_text(encoding='utf-8-sig'))]

def image_size(path):
    """Width and height from a PNG, GIF, BMP, JPEG or WebP header, or None."""
    try:
        with open(path,'rb') as f:head=f.read(65536)
    except OSError:return None
    try:
        if head[:8]==b'\x89PNG\r\n\x1a\n':return struct.unpack('>II',head[16:24])
        if head[:6] in (b'GIF87a',b'GIF89a'):return struct.unpack('<HH',head[6:10])
        if head[:2]==b'BM':w,h=struct.unpack('<ii',head[18:26]);return abs(w),abs(h)
        if head[:4]==b'RIFF' and head[8:12]==b'WEBP':
            kind=head[12:16]
            if kind==b'VP8X':return 1+int.from_bytes(head[24:27],'little'),1+int.from_bytes(head[27:30],'little')
            if kind==b'VP8 ':w,h=struct.unpack('<HH',head[26:30]);return w&0x3fff,h&0x3fff
            if kind==b'VP8L':b=int.from_bytes(head[21:25],'little');return (b&0x3fff)+1,((b>>14)&0x3fff)+1
        if head[:2]==b'\xff\xd8':
            i=2
            while i+9<len(head):
                if head[i]!=0xff:i+=1;continue
                marker=head[i+1];length=struct.unpack('>H',head[i+2:i+4])[0]
                if marker in (0xc0,0xc1,0xc2,0xc3,0xc5,0xc6,0xc7,0xc9,0xca,0xcb,0xcd,0xce,0xcf):
                    h,w=struct.unpack('>HH',head[i+5:i+9]);return w,h
                i+=2+length
    except struct.error:pass
    return None

def obs_color(value,alpha=255):
    """OBS stores colours as 0xAABBGGRR."""
    v=int(value or 0)&0xffffffff
    a=(v>>24)&255 if v>>24 else alpha
    return [v&255,(v>>8)&255,(v>>16)&255,a]

def db(volume):
    return max(-100,min(12,20*math.log10(max(float(volume),0.00001))))

# Firebot variables with a Rosorah equivalent.
VARIABLES=[('$userDisplayName','{user}'),('$username','{user}'),('$user','{user}'),('$target','{user}'),
    ('$arg[all]','{message}'),('$args','{message}'),('$chatMessage','{message}'),('$message','{message}'),
    ('$cheerBitsAmount','{bits}'),('$bits','{bits}'),('$raidViewerCount','{viewers}'),('$viewerCount','{viewers}'),
    ('$donationAmountFormatted','{amount} {currency}'),('$donationAmount','{amount}'),('$giftCount','{count}'),
    ('$giftSubCount','{count}'),('$subMonths','{count}'),('$rewardName','{reward}'),('$platform','{platform}')]

def run(request):
    project=request['project'];project['sources']=[];project['scenes']=[];project['rules']=[];project['audio']=[]
    prefs=project['preferences']
    prefs['workspace']={}
    prefs['auto_game_capture']=False
    prefs['auto_game_audio']=False
    prefs['scene_audio']={}
    prefs['chroma_keys']={}
    warnings=[];missing=[];index={};relocate=request.get('media_folder','')
    if relocate:
        root=Path(relocate)
        for i,p in enumerate(root.rglob('*')):
            if i>20000:raise ValueError('Media folder has more than 20,000 entries; choose a smaller folder')
            if p.is_file():index.setdefault(p.name.lower(),[]).append(p)
    def ident():
        i=project['next_id'];project['next_id']+=1;return i
    def media(path):
        if not path:return ''
        if Path(path).is_file():return path
        matches=index.get(path.replace('\\','/').split('/')[-1].lower(),[])
        if len(matches)==1:return str(matches[0])
        missing.append(path);return path
    def source(name,kind):
        i=ident();project['sources'].append(dict(id=i,name=name,kind=kind));return i
    def layer(i,rect=None,**kw):
        return dict(source=i,rect=rect or [0,0,1,1],visible=kw.get('visible',True),locked=kw.get('locked',False),opacity=kw.get('opacity',1),crop=kw.get('crop',[0]*4),group=kw.get('group',0))
    def audio(i,name,volume=1,muted=False,gain=0,noise='Off',monitor=False):
        project['audio'].append(dict(id=ident(),name=name,media_source=i,gain_db=max(-100,min(12,db(volume)+gain)),muted=bool(muted),tracks=[True,False,False,False,False,False],delay_ms=0,monitor=monitor,monitor_only=False,noise_suppression=noise))
    alerts_scene=dict(id=ident(),name='Imported alerts',layers=[])
    alert_layers=[]
    W,H,FPS=1920,1080,60
    docs=documents(request['path']);parsed=[];profiles=[]
    for name,text in docs:
        if name.lower().endswith('.ini'):
            c=configparser.ConfigParser(interpolation=None,strict=False)
            try:c.read_string(text)
            except configparser.Error:warnings.append(f'{name}: unreadable settings skipped');continue
            if c.has_section('Video'):
                profiles.append(name)
                if len(profiles)>1:continue
                W=c.getint('Video','BaseCX',fallback=W);H=c.getint('Video','BaseCY',fallback=H)
                kind=c.getint('Video','FPSType',fallback=0)
                try:
                    if kind==1:FPS=c.getint('Video','FPSInt',fallback=FPS)
                    elif kind==2:FPS=round(c.getint('Video','FPSNum',fallback=60)/max(1,c.getint('Video','FPSDen',fallback=1)))
                    else:FPS=round(float(c.get('Video','FPSCommon',fallback=str(FPS))))
                except ValueError:pass
                bitrate=c.get('SimpleOutput','VBitrate',fallback='') or c.get('AdvOut','FFVBitrate',fallback='')
                note=f' Its stream bitrate was {bitrate} kbps; set it in Broadcast.' if bitrate else ''
                warnings.append(f'OBS profile {PurePosixPath(name).parent.name or name}: canvas size and FPS imported.{note} Encoder, hotkeys and stream keys are not imported.')
        else:
            try:parsed.append((name,json.loads(text)))
            except (ValueError,RecursionError):warnings.append(f'{name}: invalid JSON skipped')
    if len(profiles)>1:warnings.append(f'{len(profiles)} OBS profiles found; the canvas size came from {profiles[0]}.')
    if not (16<=W<=4096 and 16<=H<=4096 and 1<=FPS<=120):raise ValueError('Unsupported canvas dimensions or FPS')
    recognized=bool(profiles)
    canvas_id=project['canvases'][0]['id']

    # ---------------- OBS scene collections ----------------
    collections=[(n,d) for n,d in parsed if isinstance(d,dict) and isinstance(d.get('sources'),list) and any(isinstance(s,dict) and s.get('id')=='scene' for s in d['sources'])]
    many=len(collections)>1
    if many:warnings.append(f'{len(collections)} OBS scene collections found; scene names start with their collection name.')
    have_desktop=False
    for filename,data in collections:
        recognized=True
        prefix=(data.get('name') or PurePosixPath(filename).stem)+' · ' if many else ''
        all_sources=[s for s in data['sources']+data.get('groups',[]) if isinstance(s,dict)]
        by_name={s.get('name'):s for s in all_sources};by_uuid={s.get('uuid'):s for s in all_sources if s.get('uuid')}
        made={};sizes={};hotkeys=0
        def find(item):
            return by_uuid.get(item.get('source_uuid')) or by_name.get(item.get('name'))
        def convert(s):
            """Creates the Rosorah source for one OBS source once; returns (id, (width, height)) or None."""
            key=s.get('uuid') or s.get('name')
            if key in made:return made[key]
            name=s.get('name','Source');kind=s.get('versioned_id') or s.get('id','');v=s.get('settings',{}) or {}
            base=s.get('id',kind);out=None;size=(W,H);has_audio=False;kind_note=None
            if base=='image_source':
                path=media(v.get('file',''));out={'Image':{'path':path}};size=image_size(path) or (W,H)
                if not image_size(path):warnings.append(f'{name}: image size unknown; check its placement')
            elif base in ('ffmpeg_source','vlc_source'):
                path=v.get('local_file','') if base=='ffmpeg_source' else next((p.get('value','') for p in v.get('playlist',[]) if isinstance(p,dict)),'')
                if base=='ffmpeg_source' and v.get('is_local_file') is False:
                    warnings.append(f'{name}: network media ({v.get("input","")}) is not supported; omitted');made[key]=None;return None
                if base=='vlc_source' and len(v.get('playlist',[]))>1:warnings.append(f'{name}: VLC playlist imported as its first file')
                out={'VideoFile':{'path':media(path),'looped':bool(v.get('looping',v.get('loop',False))),'enabled':True,'audio':True}};has_audio=True
                size=image_size(path) or (W,H)
            elif base.startswith('text_'):
                font=v.get('font',{}) or {};pt=int(font.get('size',36) or 36);flags=int(font.get('flags',0) or 0)
                text=v.get('text','')
                if v.get('read_from_file'):warnings.append(f'{name}: text read from a file is imported as its current text only')
                if base.startswith('text_ft2'):
                    color=obs_color(v.get('color1',0xffffffff));outline=bool(v.get('outline'));shadow=bool(v.get('drop_shadow'))
                    outline_width=2 if outline else 0;outline_color=[0,0,0,255];background=[0,0,0,0]
                else:
                    color=obs_color(v.get('color',0xffffff),round(int(v.get('opacity',100))*2.55))
                    outline=bool(v.get('outline'));outline_width=int(v.get('outline_size',2)) if outline else 0
                    outline_color=obs_color(v.get('outline_color',0),round(int(v.get('outline_opacity',100))*2.55));shadow=False
                    background=obs_color(v.get('bk_color',0),round(int(v.get('bk_opacity',0))*2.55))
                lines=max(1,text.count('\n')+1);longest=max((len(l) for l in text.split('\n')),default=1)
                if v.get('extents'):size=(int(v.get('extents_cx',100)),int(v.get('extents_cy',100)))
                else:size=(max(16,int(longest*pt*0.58)+outline_width*2+24),max(16,int(lines*pt*1.25)+outline_width*2+24))
                face=font.get('face','')
                out={'Widget':{'kind':'Text','label':'','content':text,'font_size':max(8,min(600,pt)),'bold':bool(flags&1),
                    'text_font':('face:'+face+(' bold' if flags&1 else '')+(' italic' if flags&2 else '')) if face else '',
                    'foreground':color,'background':background,'outline':outline_color,'outline_width':min(24,outline_width),'shadow':shadow,
                    'align':{'center':'Center','right':'Right'}.get(str(v.get('align','left')).lower(),'Left'),
                    'layout_width':max(64,min(3840,size[0])),'layout_height':max(32,min(2160,size[1])),'radius':0}}
            elif base in ('color_source','color_source_v2','color_source_v3'):
                out={'Color':obs_color(v.get('color',0xffffffff))};size=(int(v.get('width',W)),int(v.get('height',H)))
            elif base=='browser_source':
                url=v.get('url','')
                if v.get('is_local_file'):
                    warnings.append(f'{name}: local HTML browser source is not supported; omitted');made[key]=None;return None
                if not url.startswith(('https://','http://')):
                    warnings.append(f'{name}: browser source has no web address; omitted');made[key]=None;return None
                out={'Browser':{'url':url}};size=(int(v.get('width',800)),int(v.get('height',600)))
                if v.get('css','').strip() and 'background-color: rgba(0, 0, 0, 0)' not in v.get('css',''):warnings.append(f'{name}: custom browser CSS is not imported')
            elif base in ('window_capture','monitor_capture','display_capture','screen_capture','xcomposite_input','xshm_input','pipewire-desktop-capture-source','pipewire-window-capture-source','pipewire-screen-capture-source','macos-screen-capture'):
                out='ScreenCapture';kind_note=f'{name}: {base.replace("_"," ")} imported as a screen/game capture; choose what to capture in its properties'
            elif base in ('vkcapture-source','game_capture'):
                # Rosorah's automatic game capture replaces OBS's game capture hooks.
                out='ScreenCapture';prefs['auto_game_capture']=True
                kind_note=f'{name}: game capture imported; Rosorah captures supported games automatically'
            elif base=='droidcam_obs' and v.get('cur_dev_ip'):
                out={'VideoCamera':{'network':True,'host':str(v['cur_dev_ip']),'port':4747}}
                kind_note=f'{name}: DroidCam imported with its phone address; check the connection in its properties'
            elif base in ('dshow_input','v4l2_input','av_capture_input','av_capture_input_v2','macos-avcapture','droidcam_obs'):
                out={'VideoCamera':{}};has_audio=False
                res=str(v.get('resolution',''))
                if 'x' in res:
                    try:size=tuple(int(x) for x in res.split('x')[:2])
                    except ValueError:pass
                kind_note=f'{name}: camera imported; choose the device in its properties'
            elif base in ('scene','group'):
                made[key]=None;return None
            elif base in ('wasapi_input_capture','pulse_input_capture','alsa_input_capture','coreaudio_input_capture','jack_input_client','pipewire_audio_input_capture',
                          'wasapi_output_capture','pulse_output_capture','coreaudio_output_capture','pipewire_audio_output_capture',
                          'wasapi_process_output_capture','pipewire_audio_application_capture'):
                audio_channel(s);made[key]=None;return None
            elif base=='slideshow' or base=='slideshow_v2':
                files=[f.get('value','') for f in v.get('files',[]) if isinstance(f,dict)]
                if files:
                    path=media(files[0]);out={'Image':{'path':path}};size=image_size(path) or (W,H)
                    warnings.append(f'{name}: slideshow imported as its first image')
            else:
                warnings.append(f'{name}: unsupported OBS source {base}; omitted');made[key]=None;return None
            if kind_note:warnings.append(kind_note)
            i=source(name,out);made[key]=(i,size)
            gain=0;noise='Off'
            for f in s.get('filters',[]) or []:
                fid=f.get('id','');fs=f.get('settings',{}) or {}
                if not f.get('enabled',True):continue
                if fid.startswith(('chroma_key_filter','color_key_filter')):
                    preset=fs.get('key_color_type','green')
                    color={'green':[0,255,0],'blue':[0,0,255],'magenta':[255,0,255],'red':[255,0,0]}.get(preset) or obs_color(fs.get('key_color',0x00ff00))[:3]
                    v2=fid.endswith('_v2')
                    prefs['chroma_keys'][str(i)]={'enabled':True,'color':color,
                        'similarity':max(0.01,min(1,float(fs.get('similarity',80 if v2 else 400))/1000)),
                        'smoothness':max(0,min(1,float(fs.get('smoothness',50 if v2 else 80))/1000)),
                        'spill':max(0,min(1,float(fs.get('spill',100))/1000))}
                elif fid=='gain_filter':gain+=float(fs.get('db',0))
                elif fid.startswith('noise_suppress_filter'):noise='Strong' if fs.get('method','speex')!='speex' else 'Gentle'
                elif fid.startswith('crop_filter'):sizes[i]=('crop',fs)
                elif fid.startswith('color_filter') and 'opacity' in fs:sizes.setdefault(('opacity',i),fs['opacity'])
                else:warnings.append(f'{name}: OBS filter {f.get("name") or fid} has no Rosorah equivalent; omitted')
            if has_audio:audio(i,name,s.get('volume',1),s.get('muted',False),gain,noise)
            return made[key]
        def audio_channel(s,fallback=None):
            nonlocal have_desktop
            name=s.get('name') or fallback or 'Audio';base=s.get('id','')
            app=base in ('wasapi_process_output_capture','pipewire_audio_application_capture')
            output='output' in base or app
            if output and not app:
                if have_desktop:return
                have_desktop=True;name='Desktop audio'
            elif app:
                warnings.append(f'{name}: application audio imported as a channel; choose the app in the mixer')
            elif 'mic' not in name.lower():name=f'Mic · {name}'
            gain=sum(float((f.get('settings') or {}).get('db',0)) for f in s.get('filters',[]) or [] if f.get('id')=='gain_filter')
            noise=next(('Strong' if (f.get('settings') or {}).get('method','speex')!='speex' else 'Gentle' for f in s.get('filters',[]) or [] if f.get('id','').startswith('noise_suppress_filter')),'Off')
            project['audio'].append(dict(id=ident(),name=name,media_source=None,gain_db=max(-100,min(12,db(s.get('volume',1))+gain)),muted=bool(s.get('muted',False)),tracks=[True,False,False,False,False,False],delay_ms=0,monitor=False,monitor_only=False,noise_suppression=noise))
            if not output:warnings.append(f'{name}: choose its input device in the mixer')
        for key in ['DesktopAudioDevice1','DesktopAudioDevice2','AuxAudioDevice1','AuxAudioDevice2','AuxAudioDevice3','AuxAudioDevice4']:
            if isinstance(data.get(key),dict):audio_channel(data[key],key)
        def place(item,space_w,space_h,depth,group,out):
            """Appends Rosorah layers for one OBS scene item, flattening nested scenes and groups."""
            s=find(item)
            if not s or depth>8:return
            pos=item.get('pos',{});scale=item.get('scale',{});bounds=item.get('bounds',{})
            sx,sy=scale.get('x',1),scale.get('y',1)
            if item.get('rot',0) or sx<0 or sy<0:warnings.append(f'{item.get("name") or s.get("name")}: rotation/flip needs manual adjustment')
            base=s.get('id','')
            if base in ('scene','group'):
                inner_w,inner_h=(W,H) if base=='scene' else (int((s.get('settings') or {}).get('cx',W)),int((s.get('settings') or {}).get('cy',H)))
            else:
                converted=convert(s)
                if not converted:return
                i,(inner_w,inner_h)=converted
            cl,cr,ct,cb=(item.get('crop_left',0),item.get('crop_right',0),item.get('crop_top',0),item.get('crop_bottom',0))
            vw,vh=max(1,inner_w-cl-cr),max(1,inner_h-ct-cb)
            if item.get('bounds_type',0):width,height=bounds.get('x',vw),bounds.get('y',vh)
            else:width,height=vw*abs(sx),vh*abs(sy)
            x=pos.get('x',0);y=pos.get('y',0);align=item.get('align',5)
            if not align&1:x-=width if align&2 else width/2
            if not align&4:y-=height if align&8 else height/2
            rect=[x/space_w,y/space_h,max(.001,width/space_w),max(.001,height/space_h)]
            visible=item.get('visible',True)
            if base in ('scene','group'):
                children=[]
                for child in (s.get('settings') or {}).get('items',[]):
                    place(child,inner_w,inner_h,depth+1,group or (ident() if base=='group' else 0),children)
                for l in children:
                    r=l['rect'];l['rect']=[rect[0]+r[0]*rect[2],rect[1]+r[1]*rect[3],r[2]*rect[2],r[3]*rect[3]];l['visible']=l['visible'] and visible
                    out.append(l)
                return
            crop=[cl/inner_w,ct/inner_h,cr/inner_w,cb/inner_h]
            extra=sizes.get(i)
            if extra and extra[0]=='crop':
                fs=extra[1]
                if fs.get('relative',True):crop=[crop[0]+fs.get('left',0)/inner_w,crop[1]+fs.get('top',0)/inner_h,crop[2]+fs.get('right',0)/inner_w,crop[3]+fs.get('bottom',0)/inner_h]
            crop=[max(0,min(.95,c)) for c in crop]
            opacity=sizes.get(('opacity',i),1)
            opacity=opacity/100 if isinstance(opacity,(int,float)) and opacity>1 else opacity
            out.append(layer(i,rect,visible=visible,locked=item.get('locked',False),crop=crop,opacity=max(0,min(1,float(opacity))),group=group))
        order=[o.get('name') for o in data.get('scene_order',[]) if isinstance(o,dict)]
        scenes=[s for s in data['sources'] if isinstance(s,dict) and s.get('id')=='scene']
        scenes.sort(key=lambda s:order.index(s.get('name')) if s.get('name') in order else len(order))
        for s in scenes:
            hotkeys+=sum(len(v) for v in (s.get('hotkeys') or {}).values() if isinstance(v,list))
            sc=dict(id=ident(),name=prefix+s.get('name','Scene'),layers=[],obs_name=s.get('name'))
            for item in (s.get('settings') or {}).get('items',[]):place(item,W,H,0,0,sc['layers'])
            project['scenes'].append(sc)
        for s in all_sources:
            hotkeys+=sum(len(v) for v in (s.get('hotkeys') or {}).values() if isinstance(v,list))
        if hotkeys:warnings.append(f'{hotkeys} OBS hotkeys were not imported; set shortcuts again in automations')
        current=data.get('current_scene')
        if current and not many:
            for sc in project['scenes']:
                if sc.get('obs_name')==current:project['scenes'].remove(sc);project['scenes'].insert(0,sc);break
        transitions={t.get('name'):t for t in data.get('transitions',[]) if isinstance(t,dict)}
        chosen=transitions.get(data.get('current_transition'),{'id':{'Fade':'fade_transition','Cut':'cut_transition'}.get(data.get('current_transition'),'')})
        tid=chosen.get('id','');ts=chosen.get('settings',{}) or {}
        prefs['transition_ms']=max(50,min(5000,int(data.get('transition_duration',300) or 300)))
        if tid=='fade_transition':prefs['transition']='Fade'
        elif tid=='cut_transition':prefs['transition']='Cut'
        elif tid in ('swipe_transition','slide_transition','wipe_transition'):prefs['transition']='Wipe'
        elif tid=='obs_stinger_transition' and ts.get('path'):
            path=media(ts['path'])
            i=source('Stinger · '+PurePosixPath(path.replace('\\','/')).name,{'VideoFile':{'path':path,'audio':True,'looped':False,'automation_only':True}})
            project['audio'].append(dict(id=ident(),name='Stinger',media_source=i,gain_db=0,muted=False,tracks=[True,False,False,False,False,False],delay_ms=0,monitor=False,monitor_only=False,noise_suppression='Off'))
            point=int(ts.get('transition_point',500) or 500)
            if ts.get('transition_point_type',0)==1:point=round(point*1000/max(1,FPS))
            prefs['transition']='Stinger';prefs.setdefault('transition_media',{}).update(stinger=i,cut_ms=max(0,min(10000,point)))
            if ts.get('track_matte_enabled'):warnings.append('Stinger track matte is not supported; the stinger plays without it')
        elif tid:
            prefs['transition']='Fade';warnings.append(f'OBS transition {chosen.get("name") or tid} imported as a fade')
        if len(transitions)>1:warnings.append('Only the active OBS transition is imported; Rosorah uses one transition for all scene changes')
    for sc in project['scenes']:sc.pop('obs_name',None)

    # ---------------- Firebot ----------------
    def firebot(name):
        lower=name.lower()
        return any(x in lower for x in ['event','command','reward','counter','preset','timer','firebot','effect'])
    fb=[(n,d) for n,d in parsed if (n,d) not in collections and firebot(n)]
    presets={};counters={}
    def walk(v,path=''):
        if isinstance(v,dict):
            if 'effects' in v and isinstance(v['effects'],(dict,list)):yield v,path
            else:
                for k,x in v.items():yield from walk(x,path+'/'+k)
        elif isinstance(v,list):
            for i,x in enumerate(v):yield from walk(x,path+'/'+str(i))
    for name,data in fb:
        if 'preset' in name.lower() and isinstance(data,dict):
            for pid,p in data.items():
                if isinstance(p,dict) and 'effects' in p:presets[pid]=p
        if 'counter' in name.lower() and isinstance(data,dict):
            recognized=True
            for cid,c in data.items():
                if isinstance(c,dict) and 'value' in c and 'name' in c:
                    i=source(c['name'],{'Widget':{'kind':'Counter','label':c['name'],'initial':max(0,int(c.get('value',0) or 0)),'background':[20,26,40,210]}})
                    counters[cid]=i;alerts_scene['layers'].append(layer(i,[.02,.02,.3,.12],visible=False))
            if counters:warnings.append(f'{len(counters)} Firebot counters imported as hidden counter widgets in "Imported alerts"')
    events=[('follow','Follow'),('subs-gifted','Gift'),('community-subs-gifted','Gift'),('gift','Gift'),('sub','Subscription'),
        ('cheer','Bits'),('bits','Bits'),('kicks','Bits'),('raid','Raid'),('host','Raid'),('channel-reward','Redemption'),('redemption','Redemption'),('redeem','Redemption'),
        ('donation','Donation'),('tip','Donation'),('hype-train','HypeTrain'),('poll','Poll'),('prediction','Prediction'),
        ('viewer-arrived','FirstMessage'),('first-time','FirstTimeChatter'),('chat-message','ChatMessage'),('super-chat','SuperChat'),
        ('super-sticker','SuperSticker'),('member','Membership')]
    def platform(src):
        src=str(src or '').lower()
        if 'kick' in src:return 'Kick'
        if 'youtube' in src:return 'YouTube'
        if 'tiktok' in src:return 'TikTok'
        if any(x in src for x in ['streamlabs','tipeee','kofi','ko-fi','extralife','streamelements','paypal']):return 'Donations'
        if 'twitch' in src:return 'Twitch'
        return None
    def variables(text,name):
        text=str(text or '')
        for old,new in VARIABLES:text=text.replace(old,new)
        if '$' in text:warnings.append(f'{name}: some Firebot variables have no Rosorah equivalent; edit the text')
        return text
    switches=[];summary={'alerts':0,'rewards':0,'empty':[]}
    def effects_to_actions(effects,name,depth=0):
        if isinstance(effects,dict):effects=effects.get('list',[])
        actions=[]
        for e in effects or []:
            if not isinstance(e,dict) or e.get('active') is False:continue
            typ=e.get('type','')
            if typ=='firebot:chat':
                if e.get('whisper'):warnings.append(f'{name}: whispers are not supported; omitted');continue
                actions.append({'SendChat':{'text':variables(e.get('message',''),name)[:500],'platforms':[]}})
            elif typ=='firebot:playsound' and e.get('soundType','local')=='local':
                i=source(name+' · Sound',{'VideoFile':{'path':media(e.get('filepath','')),'audio':True,'gain':max(0,min(1,float(e.get('volume',5) or 5)/10)),'automation_only':True,'looped':False,'duration_secs':60}})
                audio(i,name+' · Sound',monitor=True);alert_layers.append(layer(i,[0,0,.01,.01]));actions.append({'PlaySound':{'source':i}})
            elif (typ=='firebot:playvideo' and e.get('videoType')=='Local Video') or typ in ('firebot:overlayalert','firebot:showtext','firebot:showImage','firebot:show-image'):
                duration=max(1,min(60,int(float(e.get('duration') or e.get('length') or 6))))
                text=variables(e.get('text',''),name)
                if typ=='firebot:playvideo':path=e.get('file','')
                elif typ in ('firebot:showImage','firebot:show-image'):path=e.get('file','') if e.get('imageType','local')=='local' else ''
                else:path=e.get('videoFile','') if e.get('mediaType')=='video' else e.get('imageFile','')
                mid=None
                if path:
                    mid=source(name+' · Media',{'VideoFile':{'path':media(path),'audio':True,'gain':max(0,min(1,float(e.get('videoVolume',e.get('volume',5)) or 5)/10)),'automation_only':True,'looped':False,'duration_secs':duration}});audio(mid,name+' · Media',monitor=True)
                box=source(name+' · Alert',{'Widget':{'kind':'AlertBox','label':name,'background':[0,0,0,0],'automation_only':True,'alert_layout':{'above':'Above','below':'Below'}.get(e.get('textPosition'),'Over')}})
                alert_layers.append(layer(box,[.15,.1,.7,.35]));actions.append({'DisplayAlert':{'source':box,'template':text[:500],'media':mid,'seconds':duration}})
                summary['alerts']+=1
            elif typ=='firebot:delay':
                try:ms=int(float(e.get('delay',0))*1000)
                except (ValueError,TypeError):ms=0
                actions.append({'Wait':{'milliseconds':max(0,min(ms,3600000))}})
            elif typ=='firebot:update-counter':
                target=counters.get(e.get('counterId'))
                try:value=int(float(e.get('value',1)))
                except (ValueError,TypeError):value=1
                if target and e.get('mode','increment')=='increment':
                    actions.append({'UpdateWidget':{'source':target,'text':'','increment':max(-1000000,min(1000000,value)),'start_timer':False}})
                else:warnings.append(f'{name}: counter update could not be converted (only "increment" is supported)')
            elif typ.endswith('obs-change-scene') or typ.endswith('obs-set-scene'):
                action={'SwitchScene':{'canvas':canvas_id,'scene':0}};switches.append((action,e.get('sceneName') or e.get('scene',''),name));actions.append(action)
            elif typ in ('firebot:sequential-effect','firebot:run-effect-list','firebot:random-effect'):
                inner=e.get('effectList') or e.get('effects')
                if typ=='firebot:run-effect-list':inner=(presets.get(e.get('presetListId')) or {}).get('effects')
                if depth<4 and inner:
                    nested=effects_to_actions(inner,name,depth+1)
                    if typ=='firebot:random-effect' and nested:
                        nested=nested[:1];warnings.append(f'{name}: random effect imported as its first choice')
                    actions.extend(nested)
                else:warnings.append(f'{name}: effect list could not be found; omitted')
            else:warnings.append(f'{name}: unsupported Firebot effect {typ}; omitted')
        return actions
    for filename,data in fb:
        lower=filename.lower()
        if 'preset' in lower or 'counter' in lower:continue
        for rule,path in walk(data):
            recognized=True
            name=rule.get('name') or (rule.get('twitchData') or {}).get('title') or rule.get('title') or rule.get('trigger') or path.split('/')[-1] or 'Imported automation'
            if not isinstance(name,str):name='Imported automation'
            if 'timer' in lower:warnings.append(f'{name}: Firebot timers are not supported; omitted');continue
            event=str(rule.get('eventId') or rule.get('type') or rule.get('event') or '').lower();command=rule.get('trigger','')
            is_command=isinstance(command,str) and command.startswith('!') or 'command' in lower
            if is_command and isinstance(command,str) and command.startswith('!'):kind='ChatCommand'
            elif 'reward' in lower:kind='Redemption'
            else:kind=next((v for k,v in events if k in event),None)
            if not kind:warnings.append(f'{name}: unsupported Firebot trigger {event or path}; omitted');continue
            actions=effects_to_actions(rule['effects'],name)
            if not actions:summary['empty'].append(name);continue
            options={}
            roles=[r for x in (rule.get('restrictionData') or {}).get('restrictions',[]) if isinstance(x,dict) for r in x.get('roleIds',[]) or []]
            if roles:options['permission']='Subscribers' if any('sub' in str(r) for r in roles) else 'Moderators' if all(str(r) in ('mod','broadcaster','moderator') for r in roles) else 'Everyone'
            cooldown=rule.get('cooldown') or {}
            if isinstance(cooldown,dict) and cooldown.get('user'):options['per_user_ms']=int(float(cooldown['user'])*1000)
            global_ms=int(float(cooldown.get('global') or 3))*1000 if isinstance(cooldown,dict) else 3000
            conditions={}
            reward=(rule.get('twitchData') or {}).get('id')
            if kind=='Redemption' and reward:conditions['reward_id']=str(reward);summary['rewards']+=1
            elif kind=='Redemption':warnings.append(f'{name}: plays for every channel point redemption; choose the reward in the automation')
            project['rules'].append(dict(id=ident(),name=name[:120],enabled=False,platform=('Twitch' if 'reward' in lower else None) if kind in ('ChatCommand','Redemption') and not rule.get('sourceId') else platform(rule.get('sourceId')),event=kind,
                command=command if kind=='ChatCommand' else '',cooldown_ms=max(0,min(global_ms,86400000)),actions=actions,options=options,conditions=conditions))
            if rule.get('aliases'):warnings.append(f'{name}: command aliases are not imported')
    if summary['alerts']:warnings.append(f"{summary['alerts']} Firebot alerts imported with their text and local media; animations, remote media and detailed styling need review")
    if summary['rewards']:warnings.append(f"{summary['rewards']} channel point rewards matched to their Twitch rewards")
    if summary['empty']:warnings.append(f"Skipped {len(summary['empty'])} Firebot items with no effects: {', '.join(summary['empty'][:8])}{'…' if len(summary['empty'])>8 else ''}")
    if fb:
        if True:warnings.append('Firebot automations are imported switched off. Check permissions, variables and timing before switching them on. Scripts, conditions, timers, quotes, currency and viewer data are not imported.')
    names={sc['name'].lower():sc['id'] for sc in project['scenes']}
    for action,target,rule_name in switches:
        scene_id=names.get(str(target).lower()) or next((i for n,i in names.items() if n.endswith(' · '+str(target).lower())),None)
        if scene_id:action['SwitchScene']['scene']=scene_id
        else:warnings.append(f'{rule_name}: scene "{target}" is not in this import; choose it in the automation')
    for rule in project['rules']:
        rule['actions']=[a for a in rule['actions'] if not ('SwitchScene' in a and not a['SwitchScene']['scene'])]
    project['rules']=[r for r in project['rules'] if r['actions']]
    if not recognized:raise ValueError('No supported OBS collection/profile or Firebot event/command definitions found')
    # Alerts and sounds play on every imported scene, as Firebot overlays did.
    alerts_scene['layers'].extend(alert_layers)
    for sc in project['scenes']:sc['layers'].extend(dict(l) for l in alert_layers)
    if alerts_scene['layers'] or not project['scenes']:project['scenes'].append(alerts_scene)
    project['canvases']=project['canvases'][:1];project['canvases'][0].update(scene=project['scenes'][0]['id'],width=W,height=H,fps=max(1,min(120,FPS)),name='Canvas 1')
    return dict(project=project,warnings=list(dict.fromkeys(warnings)),missing=list(dict.fromkeys(missing)))
if __name__=='__main__':
    try:print(json.dumps(run(json.load(sys.stdin))))
    except Exception as e:print(json.dumps({'error':str(e)}));sys.exit(1)
