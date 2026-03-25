use derive_more::{Display, From};
use std::marker::PhantomData;

#[derive(Debug, Clone, Copy)]
struct New;

#[derive(Debug, Clone, Copy)]
struct Paid;

#[derive(Debug, Clone, Copy)]
struct Shipped;

#[derive(Debug, Clone, From, Display)]
#[display("{}", _0)]
struct OrderId(u32);

#[derive(Debug, Clone, From, Display)]
#[display("{}", _0)]
struct CustomerName(String);

#[derive(Debug, Clone, From, Display)]
#[display("{:.2}", _0)]
struct Amount(f64);

#[derive(Debug, Clone)]
struct OrderData {
    id: OrderId,
    customer: CustomerName,
    amount: Amount,
}

#[derive(Debug, Clone)]
struct Order<State> {
    data: OrderData,
    state: PhantomData<State>,
}

impl Order<New> {
    fn new(id: OrderId, customer: CustomerName, amount: Amount) -> Self {
        Self {
            data: OrderData { id, customer, amount },
            state: PhantomData,
        }
    }

    fn pay(self) -> Order<Paid> {
        Order {
            data: self.data,
            state: PhantomData,
        }
    }
}

impl Order<Paid> {
    fn ship(self) -> Order<Shipped> {
        Order {
            data: self.data,
            state: PhantomData,
        }
    }
}

impl<State> Order<State> {
    fn id(&self) -> &OrderId {
        &self.data.id
    }

    fn customer(&self) -> &CustomerName {
        &self.data.customer
    }

    fn amount(&self) -> &Amount {
        &self.data.amount
    }
}

fn print_order_info<State>(order: &Order<State>) {
    println!("Order ID: {}", order.id());
    println!("Customer: {}", order.customer());
    println!("Amount: {}", order.amount());
}

fn main() {
    let new_order = Order::<New>::new(
        1001u32.into(),
        String::from("Ivan Petrenko").into(),
        2499.99f64.into(),
    );

    println!("--- New order ---");
    print_order_info(&new_order);

    let paid_order = new_order.pay();
    println!("\n--- Paid order ---");
    print_order_info(&paid_order);

    let shipped_order = paid_order.ship();
    println!("\n--- Shipped order ---");
    print_order_info(&shipped_order);

    // Этот код НЕ скомпилируется, и это как раз то, что требуется по заданию:
    // let invalid = Order::<New>::new(
    //     2002u32.into(),
    //     String::from("Maria").into(),
    //     100.0f64.into(),
    // );
    // let shipped = invalid.ship();

    // Ошибка будет потому, что метод ship() существует только для Order<Paid>.
}