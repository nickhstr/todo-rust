terraform {
  backend "s3" {
    bucket = "todo-app-tofu-state"
    key    = "cluster.tfstate"
    region = "us-east-1" # required by AWS SDK; Hetzner ignores it
    endpoints = {
      s3 = "https://nbg1.your-objectstorage.com"
    }
    skip_credentials_validation = true
    skip_metadata_api_check     = true
    skip_region_validation      = true
    skip_requesting_account_id  = true
    force_path_style            = true
    use_path_style              = true
  }
}
