use std::cell::RefCell;
use std::thread::{self, ThreadId};

use windows::Win32::UI::TextServices::{
    ITfContext, ITfDocumentMgr, ITfKeystrokeMgr, ITfSource, ITfThreadMgr,
};

pub fn run_before_drop<T>(resource: T, operation: impl FnOnce(&T)) {
    operation(&resource);
    drop(resource);
}

pub fn rollback_before_drop<T>(
    resource: T,
    unadvise_thread: impl FnOnce(&T),
    unadvise_key: impl FnOnce(&T),
) {
    unadvise_thread(&resource);
    unadvise_key(&resource);
    drop(resource);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivationToken(u64);

pub struct ActivationResources {
    pub thread_mgr: ITfThreadMgr,
    pub keystroke_mgr: ITfKeystrokeMgr,
    pub source: ITfSource,
    pub thread_sink_cookie: u32,
    pub focused_document: Option<ITfDocumentMgr>,
    pub focused_document_identity: Option<usize>,
    pub focused_context: Option<ITfContext>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FocusTicket(u64);

#[derive(Debug)]
pub struct FocusResources {
    pub document: Option<ITfDocumentMgr>,
    pub document_identity: Option<usize>,
    pub context: Option<ITfContext>,
}

pub struct DeactivationResources {
    pub client_id: u32,
    pub thread_mgr: Option<ITfThreadMgr>,
    pub keystroke_mgr: Option<ITfKeystrokeMgr>,
    pub source: Option<ITfSource>,
    pub thread_sink_cookie: Option<u32>,
    pub focused_document: Option<ITfDocumentMgr>,
    pub focused_context: Option<ITfContext>,
}

/// State owned by a TIP activation and accessed only from its STA owner thread.
pub struct ApartmentState {
    owner_thread: ThreadId,
    activation_generation: u64,
    focus_generation: u64,
    client_id: u32,
    activation_in_progress: bool,
    key_admission_enabled: bool,
    thread_mgr: Option<ITfThreadMgr>,
    keystroke_mgr: Option<ITfKeystrokeMgr>,
    source: Option<ITfSource>,
    thread_sink_cookie: Option<u32>,
    focused_document: Option<ITfDocumentMgr>,
    focused_document_identity: Option<usize>,
    focused_context: Option<ITfContext>,
    #[cfg(test)]
    test_focus_present: bool,
}

impl ApartmentState {
    pub fn new() -> Self {
        Self::new_for_owner(thread::current().id())
    }

    fn new_for_owner(owner_thread: ThreadId) -> Self {
        Self {
            owner_thread,
            activation_generation: 0,
            focus_generation: 0,
            client_id: 0,
            activation_in_progress: false,
            key_admission_enabled: false,
            thread_mgr: None,
            keystroke_mgr: None,
            source: None,
            thread_sink_cookie: None,
            focused_document: None,
            focused_document_identity: None,
            focused_context: None,
            #[cfg(test)]
            test_focus_present: false,
        }
    }

    pub fn try_with<R>(state: &RefCell<Self>, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let mut state = state.try_borrow_mut().ok()?;
        if state.owner_thread != thread::current().id() {
            return None;
        }
        Some(f(&mut state))
    }

    pub fn try_with_owned<T, R>(
        state: &RefCell<Self>,
        value: T,
        f: impl FnOnce(&mut Self, T) -> R,
    ) -> Result<R, T> {
        let mut state = match state.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => return Err(value),
        };
        if state.owner_thread != thread::current().id() {
            return Err(value);
        }
        Ok(f(&mut state, value))
    }

    pub fn is_activated(&self) -> bool {
        self.thread_mgr.is_some()
    }

    pub fn key_admission_enabled(&self) -> bool {
        self.key_admission_enabled
    }

    pub fn activation_generation(&self) -> u64 {
        self.activation_generation
    }

    pub fn focus_generation(&self) -> u64 {
        self.focus_generation
    }

    pub fn has_focus(&self) -> bool {
        #[cfg(test)]
        if self.test_focus_present {
            return true;
        }
        self.focused_document.is_some() && self.focused_context.is_some()
    }

    pub fn begin_activation(&mut self, client_id: u32) -> Option<ActivationToken> {
        if self.is_activated() || self.activation_in_progress {
            return None;
        }
        self.activation_generation = self.activation_generation.wrapping_add(1);
        self.activation_in_progress = true;
        self.client_id = client_id;
        Some(ActivationToken(self.activation_generation))
    }

    pub fn can_complete_activation(&self, token: ActivationToken) -> bool {
        self.activation_in_progress && self.activation_generation == token.0
    }

    pub fn accept_owned<T>(&self, token: ActivationToken, value: T) -> Result<T, T> {
        if self.can_complete_activation(token) {
            Ok(value)
        } else {
            Err(value)
        }
    }

    pub fn client_id(&self) -> Option<u32> {
        (self.activation_in_progress || self.is_activated()).then_some(self.client_id)
    }

    pub fn complete_activation(
        &mut self,
        token: ActivationToken,
        resources: ActivationResources,
    ) -> Result<(), ActivationResources> {
        let resources = self.accept_owned(token, resources)?;
        self.thread_mgr = Some(resources.thread_mgr);
        self.keystroke_mgr = Some(resources.keystroke_mgr);
        self.source = Some(resources.source);
        self.thread_sink_cookie = Some(resources.thread_sink_cookie);
        self.focused_document = resources.focused_document;
        self.focused_document_identity = resources.focused_document_identity;
        self.focused_context = resources.focused_context;
        self.activation_in_progress = false;
        self.key_admission_enabled = true;
        if self.focused_document.is_some() || self.focused_context.is_some() {
            self.focus_generation = self.focus_generation.wrapping_add(1);
        }
        Ok(())
    }

    pub fn abort_activation(&mut self, token: ActivationToken) {
        if !self.can_complete_activation(token) {
            return;
        }
        self.activation_generation = self.activation_generation.wrapping_add(1);
        self.focused_document_identity = None;
        self.client_id = 0;
        self.activation_in_progress = false;
        self.key_admission_enabled = false;
    }

    pub fn begin_deactivation(&mut self) -> Option<DeactivationResources> {
        self.key_admission_enabled = false;
        let was_active = self.thread_mgr.is_some() || self.activation_in_progress;
        let resources = was_active.then(|| DeactivationResources {
            client_id: self.client_id,
            thread_mgr: self.thread_mgr.take(),
            keystroke_mgr: self.keystroke_mgr.take(),
            source: self.source.take(),
            thread_sink_cookie: self.thread_sink_cookie.take(),
            focused_document: self.focused_document.take(),
            focused_context: self.focused_context.take(),
        });
        if was_active {
            self.activation_generation = self.activation_generation.wrapping_add(1);
        }
        self.client_id = 0;
        self.activation_in_progress = false;
        if was_active {
            self.focus_generation = self.focus_generation.wrapping_add(1);
        }
        resources
    }

    pub fn deactivate(&mut self) {
        let _ = self.begin_deactivation();
    }

    pub fn begin_focus_update(&self) -> FocusTicket {
        FocusTicket(self.focus_generation)
    }

    pub fn set_focus_if_current(
        &mut self,
        ticket: FocusTicket,
        resources: FocusResources,
    ) -> Result<FocusResources, FocusResources> {
        if ticket.0 != self.focus_generation {
            return Err(resources);
        }
        Ok(self.set_focus(resources))
    }

    pub fn set_focus(&mut self, resources: FocusResources) -> FocusResources {
        let old = FocusResources {
            document: std::mem::replace(&mut self.focused_document, resources.document),
            document_identity: std::mem::replace(
                &mut self.focused_document_identity,
                resources.document_identity,
            ),
            context: std::mem::replace(&mut self.focused_context, resources.context),
        };
        #[cfg(test)]
        {
            self.test_focus_present = false;
        }
        self.focus_generation = self.focus_generation.wrapping_add(1);
        old
    }

    pub fn take_focus(&mut self) -> FocusResources {
        self.set_focus(FocusResources {
            document: None,
            document_identity: None,
            context: None,
        })
    }

    pub fn clear_if_document_identity_current(
        &mut self,
        ticket: FocusTicket,
        identity: Option<usize>,
    ) -> Option<Option<FocusResources>> {
        if ticket.0 != self.focus_generation {
            return None;
        }
        Some(self.clear_if_document_identity(identity))
    }

    pub fn replace_context_if_current(
        &mut self,
        ticket: FocusTicket,
        identity: Option<usize>,
        context: Option<ITfContext>,
    ) -> Result<Option<ITfContext>, Option<ITfContext>> {
        if ticket.0 != self.focus_generation {
            return Err(context);
        }
        self.replace_context_if_document(identity, context)
    }

    pub fn replace_context_if_document(
        &mut self,
        identity: Option<usize>,
        context: Option<ITfContext>,
    ) -> Result<Option<ITfContext>, Option<ITfContext>> {
        if !self.focused_document_matches(identity) {
            return Err(context);
        }
        self.focus_generation = self.focus_generation.wrapping_add(1);
        Ok(std::mem::replace(&mut self.focused_context, context))
    }

    pub fn focused_document_matches(&self, identity: Option<usize>) -> bool {
        identity.is_some() && self.focused_document_identity == identity
    }

    pub fn clear_if_document_identity(
        &mut self,
        identity: Option<usize>,
    ) -> Option<FocusResources> {
        if self.focused_document_matches(identity) {
            let old = self.take_focus();
            return Some(old);
        }
        None
    }

    #[cfg(test)]
    fn set_focused_document_identity(&mut self, identity: Option<usize>) {
        self.focused_document_identity = identity;
    }

    #[cfg(test)]
    fn note_focus_presence(&mut self, present: bool) {
        self.test_focus_present = present;
        self.focus_generation = self.focus_generation.wrapping_add(1);
    }

    #[cfg(test)]
    fn note_context_changed(&mut self) {
        self.focus_generation = self.focus_generation.wrapping_add(1);
    }
}

impl Default for ApartmentState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_events_update_owned_focus_and_generation() {
        let mut state = ApartmentState::new();
        assert_eq!(state.focus_generation(), 0);

        state.note_focus_presence(true);
        assert!(state.has_focus());
        assert_eq!(state.focus_generation(), 1);

        state.note_focus_presence(true);
        assert_eq!(state.focus_generation(), 2);

        state.note_focus_presence(false);
        assert!(!state.has_focus());
        assert_eq!(state.focus_generation(), 3);

        state.note_context_changed();
        assert_eq!(state.focus_generation(), 4);
    }

    #[test]
    fn wrong_thread_and_reentrant_borrow_are_rejected_without_panicking() {
        use std::sync::mpsc;

        let state = RefCell::new(ApartmentState::new());
        let held = state.borrow_mut();
        assert!(ApartmentState::try_with(&state, |_| ()).is_none());
        drop(held);

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let other = RefCell::new(ApartmentState::new_for_owner(std::thread::current().id()));
            tx.send(ApartmentState::try_with(&other, |_| ()).is_some())
                .unwrap();
        })
        .join()
        .unwrap();
        assert!(rx.recv().unwrap());

        let owner = std::thread::current().id();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let foreign = RefCell::new(ApartmentState::new_for_owner(owner));
            tx.send(ApartmentState::try_with(&foreign, |_| ()).is_none())
                .unwrap();
        })
        .join()
        .unwrap();
        assert!(rx.recv().unwrap());
    }

    #[test]
    fn stale_focus_ticket_returns_new_resources_without_installing() {
        let mut state = ApartmentState::new();
        let ticket = state.begin_focus_update();
        state.note_context_changed();
        let resources = FocusResources {
            document: None,
            document_identity: Some(7),
            context: None,
        };
        let rejected = state
            .set_focus_if_current(ticket, resources)
            .expect_err("stale focus ticket must reject ownership");
        assert_eq!(rejected.document_identity, Some(7));
        assert!(!state.focused_document_matches(Some(7)));
    }

    #[test]
    fn reentrant_owned_operation_returns_value_without_drop() {
        use std::cell::Cell;
        use std::rc::Rc;

        #[derive(Debug)]
        struct Resource(Rc<Cell<bool>>);
        impl Drop for Resource {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        let dropped = Rc::new(Cell::new(false));
        let state = RefCell::new(ApartmentState::new());
        let held = state.borrow_mut();
        let returned =
            ApartmentState::try_with_owned(&state, Resource(dropped.clone()), |_, resource| {
                resource
            })
            .expect_err("reentrant borrow must return ownership");
        assert!(!dropped.get());
        drop(held);
        drop(returned);
        assert!(dropped.get());
    }

    #[test]
    fn rejected_owned_value_is_returned_without_drop_under_borrow() {
        use std::cell::Cell;
        use std::rc::Rc;

        #[derive(Debug)]
        struct Resource(Rc<Cell<bool>>);
        impl Drop for Resource {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        let dropped = Rc::new(Cell::new(false));
        let state = RefCell::new(ApartmentState::new());
        let stale = ActivationToken(u64::MAX);
        let result = ApartmentState::try_with(&state, |state| {
            state.accept_owned(stale, Resource(dropped.clone()))
        })
        .expect("borrow admitted");
        assert!(
            !dropped.get(),
            "resource dropped while RefCell borrow was active"
        );
        drop(result.expect_err("stale token must return resource"));
        assert!(dropped.get());
    }

    #[test]
    fn rejected_activation_rolls_back_thread_then_key_before_drop() {
        use std::cell::RefCell;
        use std::rc::Rc;

        struct Resource(Rc<RefCell<Vec<&'static str>>>);
        impl Drop for Resource {
            fn drop(&mut self) {
                self.0.borrow_mut().push("drop");
            }
        }

        let events = Rc::new(RefCell::new(Vec::new()));
        rollback_before_drop(
            Resource(events.clone()),
            |_| events.borrow_mut().push("thread"),
            |_| events.borrow_mut().push("key"),
        );
        assert_eq!(&*events.borrow(), &["thread", "key", "drop"]);
    }

    #[test]
    fn lifecycle_seam_runs_unadvise_before_resource_drop() {
        use std::cell::RefCell;
        use std::rc::Rc;

        struct Resource(Rc<RefCell<Vec<&'static str>>>);
        impl Drop for Resource {
            fn drop(&mut self) {
                self.0.borrow_mut().push("drop");
            }
        }

        let events = Rc::new(RefCell::new(Vec::new()));
        let resource = Resource(events.clone());
        run_before_drop(resource, |_| events.borrow_mut().push("unadvise"));
        assert_eq!(&*events.borrow(), &["unadvise", "drop"]);
    }

    #[test]
    fn document_identity_match_is_canonical_and_not_raw_interface_pointer_based() {
        let mut state = ApartmentState::new();
        state.set_focused_document_identity(Some(0x1234));
        assert!(state.focused_document_matches(Some(0x1234)));
        assert!(!state.focused_document_matches(Some(0x5678)));
        assert!(!state.focused_document_matches(None));
    }

    #[test]
    fn stale_activation_token_cannot_complete_after_deactivation() {
        let mut state = ApartmentState::new();
        let token = state.begin_activation(7).expect("first activation");
        state.deactivate();
        assert!(!state.can_complete_activation(token));
    }

    #[test]
    fn deactivation_invalidates_token_before_external_operations() {
        let mut state = ApartmentState::new();
        let token = state.begin_activation(7).expect("first activation");
        let generation_before = state.activation_generation();
        state.deactivate();
        assert!(state.activation_generation() > generation_before);
        assert!(!state.can_complete_activation(token));
    }

    #[test]
    fn activation_state_rejects_repeat_and_deactivation_is_idempotent() {
        let mut state = ApartmentState::new();
        let token = state.begin_activation(7).expect("first activation");
        assert!(state.begin_activation(8).is_none());
        assert_eq!(state.client_id(), Some(7));
        assert!(!state.key_admission_enabled());
        assert_eq!(state.activation_generation(), token.0);

        state.abort_activation(token);
        assert!(!state.is_activated());
        assert!(!state.key_admission_enabled());
        assert!(state.activation_generation() > token.0);
        let generation = state.activation_generation();
        state.deactivate();
        assert_eq!(state.activation_generation(), generation);
    }
}
