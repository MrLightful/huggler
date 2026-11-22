use super::*;
use crate::machines::{self, Target};
use crate::ui::field;

pub(super) struct Fields {
    executor: tokio::runtime::Handle,
    host: Entity<InputState>,
    port: Entity<InputState>,
    checkout: Entity<InputState>,
    connecting: bool,
}

impl Fields {
    pub fn new(
        executor: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Settings>,
    ) -> Self {
        Self {
            executor,
            host: cx.new(|cx| InputState::new(window, cx).placeholder("buildbox or user@host")),
            port: cx.new(|cx| InputState::new(window, cx).placeholder("From SSH config")),
            checkout: cx.new(|cx| {
                InputState::new(window, cx).placeholder("/home/user/projects/repository")
            }),
            connecting: false,
        }
    }
}

impl Settings {
    fn add_machine(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.machines.connecting {
            return;
        }
        let host = self.machines.host.read(cx).value().trim().to_owned();
        let port = self.machines.port.read(cx).value();
        let port = if port.trim().is_empty() {
            Ok(None)
        } else {
            port.trim().parse().map(Some)
        };
        let Ok(port) = port else {
            self.error = Some("Enter a port between 1 and 65535.".into());
            cx.notify();
            return;
        };
        let target = Target { host, port };
        let checkout = PathBuf::from(self.machines.checkout.read(cx).value().trim());
        if let Err(error) = target.validate().and_then(|()| {
            ensure!(
                checkout.is_absolute(),
                "Enter the absolute path to the remote Git checkout."
            );
            Ok(())
        }) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        self.error = None;
        self.machines.connecting = true;
        let connect = self
            .machines
            .executor
            .spawn(machines::add(target, checkout));
        cx.spawn_in(window, async move |view, cx| {
            let result = connect.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.machines.connecting = false;
                match result {
                    Ok(Ok(machine)) => {
                        let mut prefs = view.prefs.clone();
                        if let Some(saved) = prefs.machines.iter_mut().find(|saved| {
                            saved.environment == machine.environment
                                && saved.project == machine.project
                        }) {
                            *saved = machine.clone();
                        } else {
                            prefs.machines.push(machine.clone());
                        }
                        if view.persist(prefs, cx) {
                            for field in [
                                &view.machines.host,
                                &view.machines.port,
                                &view.machines.checkout,
                            ] {
                                field.update(cx, |input, cx| input.set_value("", window, cx));
                            }
                            if view.page == Page::Machines {
                                cx.emit(SettingsEvent::OpenMachine(machine));
                            }
                        }
                    }
                    Ok(Err(error)) => view.error = Some(error.to_string()),
                    Err(error) => view.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn machines_panel(&self, cx: &Context<Self>) -> Div {
        v_flex().gap_4()
            .children(self.prefs.machines.iter().enumerate().map(|(index, machine)| {
                let open = machine.clone();
                let forget = machine.clone();
                h_flex().gap_3().py_2().border_b_1().border_color(cx.theme().border).justify_between()
                    .child(v_flex().min_w_0().gap_1().child(section_title(machine.target.host.clone())).child(hint(machine.checkout.display().to_string(), cx)))
                    .child(Button::new(("open-machine", index)).label("Open").small().on_click(cx.listener(move |_, _, _, cx| cx.emit(SettingsEvent::OpenMachine(open.clone())))))
                    .child(Button::new(("forget-machine", index)).label("Forget").small().ghost().on_click(cx.listener(move |view, _, _, cx| {
                        let mut prefs = view.prefs.clone();
                        prefs.machines.retain(|saved| saved != &forget);
                        if view.persist(prefs, cx) {
                            cx.emit(SettingsEvent::ForgetMachine(forget.clone()));
                        }
                    })))
            }))
            .child(section_title("Add SSH machine"))
            .child(hint("Start hugglerd serve on the remote machine and confirm the host in Terminal with SSH first.", cx))
            .child(h_flex().items_start().gap_3().child(div().flex_1().child(field("SSH host or alias", &self.machines.host)))
                .child(div().w(px(150.)).child(field("SSH port (optional)", &self.machines.port))))
            .child(field("Remote Git checkout", &self.machines.checkout))
            .child(control!("Add SSH machine", Button::new("add-ssh-machine").primary()
                .label(if self.machines.connecting { "Connecting…" } else { "Add machine" })
                .disabled(self.machines.connecting)
                .on_click(cx.listener(|view, _, window, cx| view.add_machine(window, cx)))))
    }
}
