use crate::actors_manager::{ActorManagerProxyCommand, ActorsManager, Manager};
use crate::envelope::ManagerLetter;
use crate::{Actor, Handle};
use async_std::{
    sync::{Arc, Sender},
    task::spawn,
};
use dashmap::DashMap;
use futures::task::AtomicWaker;
use std::any::{Any, TypeId};
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

pub struct System {
    address_book: AddressBook,
}

impl Default for System {
    fn default() -> Self {
        System::new()
    }
}

impl System {
    pub fn new() -> System {
        let address_book = AddressBook::new();
        System { address_book }
    }

    pub fn send<A: Actor + Handle<M>, M: Debug + Send + 'static>(
        &self,
        actor_id: A::Id,
        message: M,
    ) {
        if let Some(sender) = self.address_book.get::<A>() {
            spawn(async move {
                sender
                    .send(ActorManagerProxyCommand::Dispatch(Box::new(
                        ManagerLetter::new(actor_id, message),
                    )))
                    .await;
            });
        }
    }

    pub fn stop(&self) {
        self.address_book.stop_all();
    }

    pub fn block(&self) {
        async_std::task::block_on(async {
            WaitSystemStop::new(self.address_book.clone()).await;
        });
    }
}

impl Debug for System {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ActeurSystem ()")
    }
}

impl Clone for System {
    fn clone(&self) -> Self {
        System {
            address_book: self.address_book.clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct AddressBook {
    senders: Arc<DashMap<TypeId, Box<dyn Any + Send + Sync>>>,
    managers: Arc<DashMap<TypeId, Box<dyn Manager>>>,
    waker_for_sopped_manager: Arc<AtomicWaker>,
}

impl AddressBook {
    pub fn new() -> AddressBook {
        AddressBook {
            senders: Arc::new(DashMap::new()),
            managers: Arc::new(DashMap::new()),
            waker_for_sopped_manager: Arc::new(AtomicWaker::new()),
        }
    }

    pub fn get<A>(&self) -> Option<Sender<ActorManagerProxyCommand<A>>>
    where
        A: Actor,
    {
        let type_id = TypeId::of::<A>();

        let mut sender = match self.senders.get_mut(&type_id) {
            Some(manager) => manager,
            None => {
                // TODO: Check if the creation of new actors should be really here
                self.create::<A>();
                match self.senders.get_mut(&type_id) {
                    Some(manager) => manager,
                    None => unreachable!(),
                }
            }
        };

        match sender.downcast_mut::<Sender<ActorManagerProxyCommand<A>>>() {
            Some(sender) => Some(sender.clone()),
            None => unreachable!(),
        }
    }

    pub fn add<A: Actor>(&self, sender: Sender<ActorManagerProxyCommand<A>>) {
        let type_id = TypeId::of::<A>();

        self.senders.insert(type_id, Box::new(sender));
    }

    pub fn create<A: Actor>(&self) {
        let manager = ActorsManager::<A>::new(self.clone());
        let type_id = TypeId::of::<A>();
        self.managers.insert(type_id, Box::new(manager));
    }

    pub fn stop_all(&self) {
        for manager in self.managers.iter() {
            manager.end();
        }
    }

    pub(crate) fn count_actor_managers(&self) -> usize {
        self.managers.len()
    }
}

impl Clone for AddressBook {
    fn clone(&self) -> Self {
        AddressBook {
            senders: self.senders.clone(),
            managers: self.managers.clone(),
            waker_for_sopped_manager: self.waker_for_sopped_manager.clone(),
        }
    }
}

struct WaitSystemStop(AddressBook);

impl WaitSystemStop {
    pub fn new(system: AddressBook) -> WaitSystemStop {
        WaitSystemStop(system)
    }
}

impl Future for WaitSystemStop {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        if self.0.count_actor_managers() > 0 {
            self.0.waker_for_sopped_manager.register(cx.waker());
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}
