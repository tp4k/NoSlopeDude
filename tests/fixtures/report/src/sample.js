function orderSummary(order) {
  const total = order.price * order.quantity;
  const tax = total * 0.08;
  const shipping = total > 50 ? 0 : 5;
  const grandTotal = total + tax + shipping;
  console.log("order total", grandTotal);
  console.log("tax due", tax);
  console.log("shipping cost", shipping);
  console.log("summary complete");
  order.total = grandTotal;
  return grandTotal;
}
