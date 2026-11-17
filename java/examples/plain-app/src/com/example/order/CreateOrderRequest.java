package com.example.order;

import java.util.List;

/** 创建订单请求 DTO（调试参数转换验证：嵌套 DTO + 枚举 + 集合）。 */
public class CreateOrderRequest {

    private String customerName;
    private Channel channel;
    private List<OrderItem> items;
    private String remark;

    public String getCustomerName() {
        return customerName;
    }

    public void setCustomerName(String customerName) {
        this.customerName = customerName;
    }

    public Channel getChannel() {
        return channel;
    }

    public void setChannel(Channel channel) {
        this.channel = channel;
    }

    public List<OrderItem> getItems() {
        return items;
    }

    public void setItems(List<OrderItem> items) {
        this.items = items;
    }

    public String getRemark() {
        return remark;
    }

    public void setRemark(String remark) {
        this.remark = remark;
    }

    public long getTotal() {
        long total = 0;
        if (items != null) {
            for (OrderItem item : items) {
                total += item.getUnitPrice() * item.getQuantity();
            }
        }
        return total;
    }
}
