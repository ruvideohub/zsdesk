/// Runs a chosen row of the application group.
///
/// These rows exist because the Shell has no verb for them; everything the Shell does own
/// is picked straight out of the native menu by the Shell itself, so a Shell row never
/// arrives here.
fn application_choice(state:&mut Worker,window:HWND,id:&str,action:Option<&MenuOutcome>,
        handlers:&[(String,windows::Win32::UI::Shell::IAssocHandler)],selected:&[PathBuf],request:&Request)->Result<MenuOutcome>{
    if let Some(index)=id.strip_prefix("openwith:") {
        state.allow(true)?;
        if let Some((_,handler))=index.parse::<usize>().ok().and_then(|index|handlers.get(index)){
            let data=aura_windows::desktop::data_object(window,&request.pidls)?;
            actions::invoke_handler(handler,&data)?;
        }
        // The row names one application; there is nothing left for the host to do.
        return Ok(MenuOutcome::NativeHandled);
    }
    match (id,action){
        // "Choose another application" raises the Shell's own picker.
        ("openwith",_)=>{
            state.allow(true)?;
            if let Some(path)=selected.first(){open_with(window,path)?;}
            Ok(MenuOutcome::NativeHandled)
        }
        ("shortcut",_)=>{
            state.allow(true)?;
            // A shortcut is named after the item it points at, so there is exactly one.
            if let Some(item)=selected.first(){
                let created=actions::create_shortcut(item)?;
                state.status.created.push(aura_windows::shell::new_menu::CreatedItem{path:created,rename:false});
            }
            Ok(MenuOutcome::ShellInvoked)
        }
        ("terminal",_)=>{
            state.allow(true)?;
            if let Some(folder)=request.folder.as_deref(){actions::open_terminal(folder)?;}
            Ok(MenuOutcome::NativeHandled)
        }
        (_,Some(MenuOutcome::Compress))=>{
            state.allow(true)?;
            let folder=request.folder.clone()
                .or_else(||selected.first().and_then(|path|path.parent().map(Path::to_path_buf)))
                .context("压缩目标目录缺失")?;
            let created=actions::compress(selected,&folder)?;
            state.status.created.push(aura_windows::shell::new_menu::CreatedItem{path:created,rename:false});
            Ok(MenuOutcome::ShellInvoked)
        }
        // Copying a path and unpacking an archive are the host's: it owns the clipboard and
        // the archive writer, and the status it returns is displayed there.
        (_,Some(action))=>{
            state.allow(true)?;
            Ok(*action)
        }
        _=>Ok(MenuOutcome::Dismissed),
    }
}

