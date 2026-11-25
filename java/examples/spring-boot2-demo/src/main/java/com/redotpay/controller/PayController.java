package com.redotpay.controller;

import com.redotpay.manager.ChannelManager;
import com.redotpay.service.PriceService;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RequestParam;
import org.springframework.web.bind.annotation.RestController;

/** 控制器（触发上下文访问，供 agent 捕获）。 */
@RestController
public class PayController {

    private final PriceService priceService;
    private final ChannelManager channelManager;

    public PayController(PriceService priceService, ChannelManager channelManager) {
        this.priceService = priceService;
        this.channelManager = channelManager;
    }

    @GetMapping("/quote")
    public String quote(@RequestParam(defaultValue = "BTC") String symbol) {
        return symbol + "=" + priceService.quote(symbol)
                + " channels=" + channelManager.channels().size();
    }

    @GetMapping("/ping")
    public String ping() {
        return "pong";
    }
}