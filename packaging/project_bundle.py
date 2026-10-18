"""Bounded, private local project archive transfer. No account or network access."""
import json, os, pathlib, shutil, stat, sys, uuid, zipfile

def run(data):
    path=pathlib.Path(data['path'])
    if data['mode']=='export':
        project=data['project']; media=[]
        for source in project['sources']:
            kind=source['kind']
            if isinstance(kind,dict):
                settings=kind.get('Image') or kind.get('VideoFile')
                if settings is not None:
                    original=pathlib.Path(settings['path'])
                    if not original.is_file(): raise ValueError('Locate missing media before exporting: '+source['name'])
                    suffix=original.suffix.lower()
                    if len(suffix)>12 or any(c not in '.abcdefghijklmnopqrstuvwxyz0123456789' for c in suffix): suffix='.bin'
                    entry='media/'+str(source['id'])+suffix
                    media.append((original,entry));settings['path']=entry
        if path.exists():raise ValueError('Choose a new filename; existing bundles are not overwritten.')
        temporary=path.with_name(path.name+'.partial-'+uuid.uuid4().hex)
        try:
            with zipfile.ZipFile(temporary,'x',compression=zipfile.ZIP_DEFLATED,compresslevel=1) as archive:
                archive.writestr('project.json',json.dumps(project,ensure_ascii=False))
                for original,entry in media: archive.write(original,entry)
            os.link(temporary,path)
        finally:
            temporary.unlink(missing_ok=True)
        return str(path)
    parent=pathlib.Path(data['parent']);folder=parent/('Rosorah-import-'+uuid.uuid4().hex[:10])
    folder.mkdir(mode=0o700)
    try:
        with zipfile.ZipFile(path) as archive:
            files=archive.infolist();names=[i.filename for i in files]
            if len(files)>5000 or len(set(names))!=len(names):raise ValueError('Bundle has too many or duplicate entries.')
            total=sum(i.file_size for i in files)
            if total>50*1024**3 or total>shutil.disk_usage(parent).free:raise ValueError('Bundle exceeds the available space or 50 GB import limit.')
            for info in files:
                p=pathlib.PurePosixPath(info.filename)
                valid=info.filename=='project.json' or (len(p.parts)==2 and p.parts[0]=='media' and p.parts[1] not in ('.','..'))
                if not valid or '\\' in info.filename or p.is_absolute() or stat.S_ISLNK(info.external_attr>>16):raise ValueError('Bundle contains an unsafe entry.')
            if archive.getinfo('project.json').file_size>16*1024**2:raise ValueError('Project is too large.')
            project=json.loads(archive.read('project.json'))
            for source in project.get('sources',[]):
                kind=source.get('kind')
                if isinstance(kind,dict):
                    settings=kind.get('Image') or kind.get('VideoFile')
                    if settings is not None:
                        rel=settings.get('path','')
                        if rel not in names or not rel.startswith('media/'):raise ValueError('Project references media outside this bundle.')
                        settings['path']=str(folder/rel)
            for info in files:
                if info.filename=='project.json':continue
                output=folder/info.filename;output.parent.mkdir(exist_ok=True)
                with archive.open(info) as source,output.open('xb') as target:shutil.copyfileobj(source,target,1024*1024)
            target=folder/'project.json';target.write_text(json.dumps(project,ensure_ascii=False),encoding='utf-8')
            return str(target)
    except Exception:
        shutil.rmtree(folder);raise

if __name__=='__main__':
    try: print(json.dumps({'path':run(json.load(sys.stdin))}))
    except Exception as error: print(json.dumps({'error':str(error)}));sys.exit(1)
