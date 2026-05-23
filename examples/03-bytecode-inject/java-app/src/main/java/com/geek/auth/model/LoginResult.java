package com.geek.auth.model;

public class LoginResult {
    private boolean success;
    private String token;
    private String message;

    public static LoginResult ok(String token) {
        LoginResult r = new LoginResult();
        r.success = true;
        r.token = token;
        r.message = "login success";
        return r;
    }

    public static LoginResult fail(String message) {
        LoginResult r = new LoginResult();
        r.success = false;
        r.message = message;
        return r;
    }

    public boolean isSuccess() { return success; }
    public void setSuccess(boolean success) { this.success = success; }
    public String getToken() { return token; }
    public void setToken(String token) { this.token = token; }
    public String getMessage() { return message; }
    public void setMessage(String message) { this.message = message; }
}
